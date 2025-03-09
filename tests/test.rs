use pretty_assertions::assert_eq;

use std::path::Path;

use rust_log_analyzer as rla;

const TEST_LOCATION: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests");

#[test]
fn test_missing_line() {
    let log = std::fs::read(Path::new(TEST_LOCATION).join("test_missing_line.txt")).unwrap();
    let lines = rla::sanitize::split_lines(&log)
        .iter()
        .map(|l| {
            rla::index::Sanitized(rla::sanitize::clean(
                &rla::ci::GitHubActions::new("DUMMY_TOKEN"),
                l,
            ))
        })
        .collect::<Vec<_>>();
    let blocks = rla::extract::extract(
        &rla::extract::Config::default(),
        &rla::index::Index::default(),
        &lines,
    );

    let expected = r###"Current runner version: '2.320.0'
Runner name: 'ubuntu-20.04-4core-16gb_f6653e6045ce'
Runner group name: 'Default Larger Runners'
Machine name: 'runner'
##[group]Operating System
Ubuntu
20.04.6
LTS
##[endgroup]
##[group]Runner Image
Image: ubuntu-20.04
Version: 20240922.1.0
Included Software: https://github.com/actions/runner-images/blob/ubuntu20/20240922.1/images/ubuntu/Ubuntu2004-Readme.md
Image Release: https://github.com/actions/runner-images/releases/tag/ubuntu20%2F20240922.1
##[endgroup]
##[group]GITHUB_TOKEN Permissions
Contents: read
Metadata: read
Packages: read
##[endgroup]
Secret source: None
Prepare workflow directory
Prepare all required actions
Getting action download info
Download action repository 'msys2/setup-msys2@v2.22.0' (SHA:cc11e9188b693c2b100158c3322424c4cc1dadea)
Download action repository 'actions/checkout@v4' (SHA:eef61447b9ff4aafe5dcd4e0bbf5d482be7e7871)
Download action repository 'actions/upload-artifact@v4' (SHA:604373da6381bf24206979c74d06a550515601b9)
Complete job name: PR - mingw-check
##[group]Run git config --global core.autocrlf false
git config --global core.autocrlf false
shell: /usr/bin/bash --noprofile --norc -e -o pipefail {0}"###;

    let actual = blocks[0]
        .iter()
        .map(|line| String::from_utf8_lossy(&line.0).into_owned())
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(actual, expected);
}
