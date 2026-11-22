use super::*;
use regex::bytes::{Regex, RegexSet};
use std::io::{BufRead, BufReader, Read};
use std::sync::OnceLock;

#[derive(Serialize)]
pub struct Review {
    pub tree: String,
    pub warnings: Vec<String>,
}

pub(super) fn check_operation(dir: &Path) -> Result<()> {
    for marker in [
        "MERGE_HEAD",
        "CHERRY_PICK_HEAD",
        "REVERT_HEAD",
        "rebase-apply",
        "rebase-merge",
        "sequencer",
        "BISECT_START",
    ] {
        if git_path(dir, marker)?.exists() {
            return Err("An existing Git operation is in progress. Finish or abort it with Git before continuing.".into());
        }
    }
    Ok(())
}

fn sensitive_name(path: &str) -> bool {
    let path = path.to_ascii_lowercase();
    let name = path.rsplit('/').next().unwrap_or(&path);
    matches!(
        name,
        ".env"
            | ".npmrc"
            | ".pypirc"
            | "credentials"
            | ".git-credentials"
            | "id_rsa"
            | "id_ed25519"
            | "id_ecdsa"
            | ".netrc"
            | "kubeconfig"
            | "secrets.yaml"
            | "secrets.yml"
    ) || name.starts_with(".env.")
        || name.ends_with(".pem")
        || name.ends_with(".p12")
        || name.ends_with(".pfx")
        || name.ends_with(".key")
        || name.contains("credentials.")
}

fn patterns() -> &'static RegexSet {
    static PATTERNS: OnceLock<RegexSet> = OnceLock::new();
    PATTERNS.get_or_init(||RegexSet::new([
        r"-----BEGIN (?:[A-Z0-9 ]+ )?PRIVATE KEY-----",
        r"(?:gh[pousr]_|github_pat_)[A-Za-z0-9_]{16,}",
        r"(?:AKIA|ASIA)[A-Z0-9]{16}",
        r"(?:sk-[A-Za-z0-9_-]{20,}|xox[baprs]-[A-Za-z0-9-]{16,})",
        r#"(?i)(?:api[_-]?key|access[_-]?token|refresh[_-]?token|password|passwd|client[_-]?secret|secret[_-]?key)["']?[ \t]{0,16}[:=][ \t]{0,16}["']?[A-Za-z0-9+/_.@!-]{4,}"#,
        r"(?i)authorization:[ \t]{0,16}(?:bearer|basic)[ \t]+[A-Za-z0-9+/=_-]{10,}"
    ]).expect("valid built-in credential patterns"))
}

fn contains_sensitive(bytes: &[u8], path: &str) -> bool {
    if !patterns().is_match(bytes) {
        return false;
    }
    if !path.ends_with(".rs") {
        return true;
    }

    // Exclude unquoted Rust field types. Only test files can also have a
    // short sample in a substring assertion. Scan the rest of each line.
    static FIELD: OnceLock<Regex> = OnceLock::new();
    static ASSERTION: OnceLock<Regex> = OnceLock::new();
    let field = FIELD.get_or_init(|| {
        Regex::new(r"(?m)^[ \t]*(?:access_token|refresh_token):[ \t]*String,[ \t]*\r?$").unwrap()
    });
    let filtered = field.replace_all(bytes, &b" "[..]);
    if path
        .split('/')
        .any(|part| matches!(part, "tests" | "test" | "tests.rs"))
        || path.ends_with("_test.rs")
    {
        let assertion = ASSERTION.get_or_init(|| {
            Regex::new(r#"\.contains\("(?:authorization: bearer |refresh_token=)[a-z]{1,12}-[a-z]{1,12}"\)"#).unwrap()
        });
        return patterns().is_match(&assertion.replace_all(&filtered, &b" "[..]));
    }
    patterns().is_match(&filtered)
}

fn scan_blob(reader: &mut impl Read, mut left: u64, path: &str) -> Result<bool> {
    const OVERLAP: usize = 256;
    let mut buf = vec![0u8; 65536 + OVERLAP];
    let mut carry = 0;
    let mut found = false;
    while left > 0 {
        let count = (left.min(65536)) as usize;
        reader
            .read_exact(&mut buf[carry..carry + count])
            .map_err(|_| "Git interrupted the staged content review.")?;
        left -= count as u64;
        let end = carry + count;
        if !found {
            // Prefer breaking between lines so an ordinary Rust declaration or
            // assertion split across reads still has its full context.
            let tail = end.saturating_sub(OVERLAP);
            let boundary = buf[tail..end].iter().rposition(|&b| b == b'\n');
            let (scan_end, next) = match (left, boundary) {
                (0, _) => (end, end),
                (_, Some(i)) => (tail + i + 1, tail + i + 1),
                (_, None) => (end, tail),
            };
            found = contains_sensitive(&buf[..scan_end], path);
            buf.copy_within(next..end, 0);
            carry = end - next;
        }
    }
    Ok(found)
}

fn scan_objects(dir: &Path, items: &[(String, String)], warnings: &mut Vec<String>) -> Result<()> {
    let mut child = command()
        .current_dir(dir)
        .args(["cat-file", "--batch"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "Could not open staged objects for inspection.")?;
    let result: Result<()> = (|| {
        let mut input = child
            .stdin
            .take()
            .ok_or("Could not communicate with Git.")?;
        let mut reader = BufReader::new(
            child
                .stdout
                .take()
                .ok_or("Could not read staged Git objects.")?,
        );
        for (id, path) in items {
            writeln!(input, "{id}")
                .and_then(|_| input.flush())
                .map_err(|_| "Git stopped reading staged object IDs.")?;
            let mut header = String::new();
            reader
                .read_line(&mut header)
                .map_err(|_| "Git returned an unreadable object size.")?;
            let fields: Vec<_> = header.split_whitespace().collect();
            if fields.len() != 3 || fields[0] != id || fields[1] != "blob" {
                return Err("Could not inspect a staged file.".into());
            }
            let size = fields[2]
                .parse::<u64>()
                .map_err(|_| "Invalid staged file size.")?;
            if size > 50 * 1024 * 1024 {
                warnings.push(if size > 100 * 1024 * 1024 {
                    format!("{path} exceeds GitHub's 100 MiB file limit.")
                } else {
                    format!("{path} exceeds GitHub's 50 MiB file warning threshold.")
                });
            }
            let found = scan_blob(&mut reader, size, path)?;
            let mut sep = [0];
            reader
                .read_exact(&mut sep)
                .map_err(|_| "Incomplete staged file.")?;
            if sep != [b'\n'] {
                return Err("Invalid staged object response.".into());
            }
            if found || sensitive_name(path) {
                warnings.push(format!(
                    "{path} may contain sensitive configuration or credentials."
                ));
            }
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = child.kill();
    }
    let status = child
        .wait()
        .map_err(|_| "The staged content review was interrupted.")?;
    result?;
    if !status.success() {
        return Err("Git could not complete staged content inspection.".into());
    }
    Ok(())
}

pub fn inspect_commit(dir: &Path) -> Result<Review> {
    root(dir)?;
    check_operation(dir)?;
    let tree = line(dir, &["write-tree"])?;
    let base = match head(dir)? {
        Some(id) => id,
        None => utf8(&checked(execute(
            command()
                .current_dir(dir)
                .args(["hash-object", "-w", "-t", "tree", "--stdin"]),
            Some(b""),
        )?)?)?
        .trim()
        .to_owned(),
    };
    let raw = bytes(
        dir,
        &[
            "diff-tree",
            "-r",
            "-z",
            "--no-renames",
            "--no-commit-id",
            "--no-abbrev",
            "--raw",
            &base,
            &tree,
            "--",
        ],
    )?;
    if raw.is_empty() {
        return Err("Stage at least one change before committing.".into());
    }
    let mut fields = raw.split(|b| *b == 0);
    let mut objects = Vec::new();
    let mut warnings = Vec::new();
    while let Some(header) = fields.next() {
        if header.is_empty() {
            continue;
        }
        let parts: Vec<_> = utf8(header)?.split_whitespace().collect();
        let path = utf8(fields.next().ok_or("Missing staged filename.")?)?;
        if parts.len() != 5 {
            return Err("Git returned an unreadable staged tree.".into());
        }
        if parts[4] == "D" {
            continue;
        }
        if parts[1] == "160000" {
            warnings.push(format!(
                "{path} points to a submodule. Its contents were not inspected."
            ));
        } else {
            objects.push((parts[3].to_owned(), path.to_owned()));
        }
    }
    scan_objects(dir, &objects, &mut warnings)?;
    Ok(Review { tree, warnings })
}

pub fn commit(
    dir: &Path,
    title: &str,
    description: &str,
    reviewed_tree: &str,
    account: Option<&AccountEntry>,
) -> Result<()> {
    root(dir)?;
    check_operation(dir)?;
    let title = title.trim();
    if title.is_empty() || title.contains(['\r', '\n', '\0']) {
        return Err("Enter a single-line commit title.".into());
    }
    let author = author(dir)?.with_account(account);
    if author.name.trim().is_empty() || author.email.trim().is_empty() {
        return Err("Set a Git author name and email before committing.".into());
    }
    if line(dir, &["write-tree"])? != reviewed_tree {
        return Err(
            "The staged files changed after review. Review the current selection again.".into(),
        );
    }
    let old = head(dir)?;
    if let Some(id) = &old {
        if line(dir, &["rev-parse", &format!("{id}^{{tree}}")])? == reviewed_tree {
            return Err("There are no staged changes to commit.".into());
        }
    }
    // The review identifies an immutable tree. An isolated index prevents files
    // concurrently staged in the real index from being included in this commit.
    let temp = super::network::private_tempdir()
        .map_err(|_| "Cannot reserve a temporary commit index.")?;
    let index = temp.path().join("index");
    checked(execute(
        command()
            .current_dir(dir)
            .env("GIT_INDEX_FILE", &index)
            .args(["read-tree", reviewed_tree]),
        None,
    )?)?;
    let message = if description.trim().is_empty() {
        title.to_owned()
    } else {
        format!("{title}\n\n{}", description.trim())
    };
    checked(execute(
        command()
            .current_dir(dir)
            .env("GIT_INDEX_FILE", &index)
            // Scope the identity to this commit, so switching accounts still works offline.
            .arg("-c")
            .arg(format!("user.name={}", author.name))
            .arg("-c")
            .arg(format!("user.email={}", author.email))
            .args(["commit", "--cleanup=verbatim", "-F", "-"]),
        Some(message.as_bytes()),
    )?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flagged(data: &[u8], path: &str) -> bool {
        scan_blob(&mut &data[..], data.len() as u64, path).unwrap()
    }

    #[test]
    fn rust_declarations_and_sample_assertions_do_not_flag_app_sources() {
        assert!(!flagged(include_bytes!("../github.rs"), "src/github.rs"));
        assert!(!flagged(
            include_bytes!("../github/tests.rs"),
            "src/github/tests.rs"
        ));
        assert!(!flagged(include_bytes!("review.rs"), "src/git/review.rs"));
        let assertion = format!(r#"assert!(body.contains("{}=old-next"));"#, "refresh_token");
        let mut across = vec![b' '; 65515];
        across.push(b'\n');
        across.extend_from_slice(assertion.as_bytes());
        assert!(!flagged(&across, "tests/check.rs"));
        assert!(flagged(&across, "src/check.rs"));

        let secret = format!(r#"{{"{}":"example-value"}}"#, "refresh_token");
        assert!(flagged(
            format!("{assertion} {secret}").as_bytes(),
            "src/check_test.rs"
        ));
        let token = ["ghp", "_", &"a".repeat(30)].concat();
        let real_assertion =
            format!(r#"assert!(header.contains("authorization: bearer {token}"));"#);
        assert!(flagged(real_assertion.as_bytes(), "src/check_test.rs"));
        let key = ["-----BEGIN ", "PRIVATE KEY-----"].concat();
        assert!(flagged(key.as_bytes(), "src/check_test.rs"));
    }
}
