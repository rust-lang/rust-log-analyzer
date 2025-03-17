const START_DELIMITER: u8 = b'[';
const END_DELIMITER: u8 = b']';
const SEPARATOR: u8 = b'=';

const JOB_NAME_VARIABLE: &str = "CI_JOB_NAME";
const PR_NUMBER_VARIABLE: &str = "CI_PR_NUMBER";
/// URL pointing to a documentation page about the job.
/// Added in https://github.com/rust-lang/rust/pull/136911.
const JOB_DOC_URL: &str = "CI_JOB_DOC_URL";

pub struct LogVariables<'a> {
    pub job_name: Option<&'a str>,
    pub pr_number: Option<&'a str>,
    pub doc_url: Option<&'a str>,
}

impl<'a> LogVariables<'a> {
    pub fn extract<I: crate::index::IndexData>(lines: &'a [I]) -> Self {
        let mut result = LogVariables {
            job_name: None,
            pr_number: None,
            doc_url: None,
        };

        for line in lines {
            let sanitized = line.sanitized();

            if result.job_name.is_none() {
                result.job_name = extract_variable(sanitized, JOB_NAME_VARIABLE);
            }
            if result.pr_number.is_none() {
                result.pr_number = extract_variable(sanitized, PR_NUMBER_VARIABLE);
            }
            if result.doc_url.is_none() {
                result.doc_url = extract_variable(sanitized, JOB_DOC_URL);
            }

            // Early exit if everything was found
            if result.job_name.is_some() && result.pr_number.is_some() && result.doc_url.is_some() {
                break;
            }
        }

        result
    }
}

fn extract_variable<'a>(line: &'a [u8], name: &str) -> Option<&'a str> {
    if line.first() != Some(&START_DELIMITER) || line.last() != Some(&END_DELIMITER) {
        return None;
    }

    let equals = line.iter().position(|byte| *byte == SEPARATOR)?;
    if &line[1..equals] != name.as_bytes() {
        return None;
    }
    std::str::from_utf8(&line[equals + 1..line.len() - 1]).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::Sanitized;

    #[test]
    fn test_extract_variable() {
        assert_eq!(None, extract_variable(b"[foo=bar", "foo"));
        assert_eq!(None, extract_variable(b"foo=bar]", "foo"));
        assert_eq!(None, extract_variable(b"[foo]", "foo"));
        assert_eq!(None, extract_variable(b"[baz=bar]", "foo"));
        assert_eq!(Some("bar"), extract_variable(b"[foo=bar]", "foo"));
    }

    #[test]
    fn test_log_variables_extract() {
        const LOG: &[Sanitized<&str>] = &[
            Sanitized("foo"),
            Sanitized("bar"),
            Sanitized("[CI_JOB_NAME=test-job]"),
            Sanitized("baz"),
            Sanitized("[CI_PR_NUMBER=123]"),
            Sanitized("quux"),
            Sanitized("[CI_JOB_DOC_URL=https://github.com/rust-lang/rust/job1]"),
            Sanitized("foobar"),
        ];

        let extracted = LogVariables::extract(LOG);
        assert_eq!(Some("test-job"), extracted.job_name);
        assert_eq!(Some("123"), extracted.pr_number);
        assert_eq!(
            Some("https://github.com/rust-lang/rust/job1"),
            extracted.doc_url
        );
    }
}
