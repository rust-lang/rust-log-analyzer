use crate::index::{Index, IndexData};
use aho_corasick::AhoCorasick;
use std::iter;
use std::mem;

/// Plaintext patterns which, if found in a line, cause all remaining lines to be ignored until the
/// corresponding pattern (second tuple element) is found in a line.
static IGNORE_BLOCK: &[(&str, &str)] = &[
    // Skip environment varialbes
    (
        "##[group]Run src/ci/scripts/dump-environment.sh",
        "##[group]",
    ),
    // Skip initialization as it involves submodules, which can change often
    (
        "##[group]Run src/ci/scripts/checkout-submodules.sh",
        "##[group]",
    ),
    // Skip AWS cli installation, it involves network (so changes)
    ("##[group]Run src/ci/scripts/install-awscli.sh", "##[group]"),
    // Cargo downloads crates in unpredictable order
    ("Downloading crates ...", "Compiling"),
    // Skip all groups invoking git commands
    ("[command]/usr/bin/git", "##[endgroup]"),
    // Skip clock drift checks
    ("#[group]Clock drift check", "##[endgroup]"),
    // Skip environment variable dumps, as these can contain e.g. a SHA which is different in every
    // build.
    ("env:", "##[endgroup]"),
    // See src/ci/scripts/dump-environment.sh in rust-lang/rust
    (
        "environment variables:",
        "biggest files in the working dir:",
    ),
];

lazy_static! {
    static ref IGNORE_BLOCK_START: AhoCorasick =
        AhoCorasick::new(IGNORE_BLOCK.iter().map(|x| &x.0).cloned()).unwrap();
}

lazy_static! {
    static ref IGNORE_BLOCK_END: Vec<AhoCorasick> = IGNORE_BLOCK
        .iter()
        .map(|&s| AhoCorasick::new(iter::once(s.1)).unwrap())
        .collect();
}

pub struct Config {
    pub unique_5gram_max_index: u32,
    pub block_merge_distance: usize,
    pub block_separator_max_score: u32,
    pub unique_line_min_score: u32,
    pub block_max_lines: usize,
    pub context_lines: usize,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            unique_5gram_max_index: 10,
            block_merge_distance: 8,
            block_separator_max_score: 0,
            unique_line_min_score: 50,
            block_max_lines: 500,
            context_lines: 4,
        }
    }
}

pub fn score<I: IndexData>(config: &Config, index: &Index, line: &I) -> u32 {
    index
        .scores(line)
        .filter(|&val| val <= config.unique_5gram_max_index)
        .map(|val| config.unique_5gram_max_index - val)
        .sum()
}

enum State<'a> {
    SearchingSectionStart,
    SearchingOutlier,
    Printing,
    Ignoring(&'a AhoCorasick),
}

#[derive(Copy, Clone)]
struct Line<'i, I: IndexData + 'i> {
    score: u32,
    line: &'i I,
}

pub fn extract<'i, I: IndexData + 'i>(
    config: &Config,
    index: &Index,
    lines: &'i [I],
) -> Vec<Vec<&'i I>> {
    assert!(config.context_lines < config.block_merge_distance);

    let lines: Vec<Line<_>> = lines
        .iter()
        .map(|line| Line {
            line,
            score: score(config, index, line),
        })
        .collect();

    let mut i = 0;
    let mut state = State::SearchingSectionStart;
    let mut section_start = 0;
    let mut prev_section_end = 0;

    let mut active_block = vec![];
    let mut blocks = vec![];

    let mut trailing_context = 0;

    while i < lines.len() {
        if let Some(m) = IGNORE_BLOCK_START.find(lines[i].line.sanitized()) {
            trailing_context = 0;

            if let State::Printing = state {
                if !active_block.is_empty() {
                    blocks.push(mem::replace(&mut active_block, vec![]));
                }
            }

            state = State::Ignoring(&IGNORE_BLOCK_END[m.pattern()]);
            i += 1;
            continue;
        }

        match state {
            State::Ignoring(a) => {
                if a.find(lines[i].line.sanitized()).is_some() {
                    state = State::SearchingSectionStart;
                }

                i += 1;
                continue;
            }

            State::SearchingSectionStart => {
                if lines[i].score > config.block_separator_max_score {
                    state = State::SearchingOutlier;
                    section_start = i;
                } else {
                    if trailing_context > 0 {
                        trailing_context -= 1;
                        blocks.last_mut().unwrap().push(lines[i].line);
                        prev_section_end = i;
                    }

                    i += 1;
                    continue;
                }
            }

            State::SearchingOutlier => {
                if lines[i].score <= config.block_separator_max_score {
                    if trailing_context > 0 {
                        trailing_context -= 1;
                        blocks.last_mut().unwrap().push(lines[i].line);
                        prev_section_end = i;
                    }

                    state = State::SearchingSectionStart;
                    i += 1;
                    continue;
                }

                if lines[i].score >= config.unique_line_min_score {
                    trailing_context = 0;

                    let start_printing;

                    if prev_section_end + config.block_merge_distance >= section_start {
                        if !blocks.is_empty() {
                            let last_idx = blocks.len() - 1;
                            active_block = blocks.remove(last_idx);
                        }
                        start_printing = prev_section_end;
                    } else {
                        start_printing = section_start.saturating_sub(config.context_lines);
                    }

                    for j in start_printing..i {
                        active_block.push(lines[j].line);
                    }

                    state = State::Printing;
                } else {
                    if trailing_context > 0 {
                        trailing_context -= 1;
                        blocks.last_mut().unwrap().push(lines[i].line);
                        prev_section_end = i;

                        // No need to update section_start since we'll trigger the `merge` case above
                        // anyway (prev_section_end >= section_start).
                    }

                    i += 1;
                    continue;
                }
            }

            State::Printing => {
                if lines[i].score <= config.block_separator_max_score {
                    if !active_block.is_empty() {
                        blocks.push(mem::replace(&mut active_block, vec![]));
                    }
                    prev_section_end = i;
                    state = State::SearchingSectionStart;

                    trailing_context = config.context_lines;
                } else {
                    active_block.push(lines[i].line);
                }

                i += 1;
                continue;
            }
        }
    }

    if !active_block.is_empty() {
        blocks.push(active_block);
    }

    blocks.retain(|block| !block.is_empty());
    blocks
        .iter_mut()
        .for_each(|block| block.truncate(config.block_max_lines));

    blocks
}
