//! The repository a captured Claude Code session worked in, a small calculator with its tests, and
//! that session's payloads as the learning tests send them, their paths moved into the test's
//! repository.

#![allow(
    clippy::indexing_slicing,
    reason = "the payloads edited here are JSON objects, where indexing a missing key adds it"
)]

use std::path::{Path, PathBuf};

use serde_json::Value;

use super::{TestResult, payloads};

/// The Claude Code version whose captured payloads these tests send.
pub const CAPTURED: &str = "2.1.312";

/// The session of the captured payloads.
pub const S1: &str = "00000000-0000-4000-8000-000000000001";

/// `calc/ops.py` as the captured session found it: `divide`, on lines 12 and 13, divides by floor.
pub const OPS: &str = "\"\"\"Arithmetic helpers.\"\"\"\n\n\ndef add(a, b):\n    return a + b\n\n\n\
                   def mul(a, b):\n    return a * b\n\n\ndef divide(a, b):\n    return a // b\n";

/// `calc/ops.py` once the captured edit made `divide` divide truly, on line 13.
pub const OPS_FIXED: &str = "\"\"\"Arithmetic helpers.\"\"\"\n\n\ndef add(a, b):\n    return a + b\n\n\n\
                         def mul(a, b):\n    return a * b\n\n\ndef divide(a, b):\n    return a / b\n";

/// `tests/test_ops.py`: 14 lines testing the three functions.
pub const TESTS: &str = "import unittest\n\nfrom calc.ops import add, divide, mul\n\n\n\
                     class OpsTest(unittest.TestCase):\n    def test_add(self):\n        \
                     self.assertEqual(add(2, 3), 5)\n\n    def test_mul(self):\n        \
                     self.assertEqual(mul(2, 3), 6)\n\n    def test_divide(self):\n        \
                     self.assertEqual(divide(7, 2), 3.5)\n";

/// `main.py`: 3 lines that print a sum.
pub const MAIN: &str = "from calc.ops import add\n\nprint(add(1, 2))\n";

/// A home folder holding the repository the payloads were captured in: `calc/ops.py` and the
/// tests the session ran, `main.py` and the two packages' empty `__init__.py`, which the captured
/// search for Python files lists.
pub fn repository() -> TestResult<(tempfile::TempDir, PathBuf)> {
    let home = tempfile::tempdir()?;
    let repo = home.path().join("repo");
    std::fs::create_dir_all(repo.join("calc"))?;
    std::fs::create_dir_all(repo.join("tests"))?;
    std::fs::write(repo.join("calc/__init__.py"), "")?;
    std::fs::write(repo.join("calc/ops.py"), OPS)?;
    std::fs::write(repo.join("tests/__init__.py"), "")?;
    std::fs::write(repo.join("tests/test_ops.py"), TESTS)?;
    std::fs::write(repo.join("main.py"), MAIN)?;
    Ok((home, repo))
}

/// The captured payload `name`, its paths moved under `root`.
pub fn captured(name: &str, root: &Path) -> TestResult<Value> {
    Ok(serde_json::from_slice(&payloads::payload(
        CAPTURED, name, root,
    )?)?)
}

/// The captured read of `calc/ops.py` made a read of the whole of `tests/test_ops.py`.
pub fn read_of_tests(root: &Path) -> TestResult<Value> {
    let mut read = captured("post-tool-use-read.json", root)?;
    let path = root.join("tests").join("test_ops.py");
    let path = Value::String(path.to_string_lossy().into_owned());
    read["tool_input"]["file_path"] = path.clone();
    let file = &mut read["tool_response"]["file"];
    file["filePath"] = path;
    file["content"] = Value::String(TESTS.to_owned());
    file["numLines"] = Value::from(15);
    file["totalLines"] = Value::from(15);
    Ok(read)
}

/// The captured edit of line 13 of `calc/ops.py` made the edit that puts it back.
pub fn edit_back(root: &Path) -> TestResult<Value> {
    let mut edit = captured("post-tool-use-edit.json", root)?;
    let (fixed, floor) = ("    return a / b", "    return a // b");
    edit["tool_input"]["old_string"] = Value::from(fixed);
    edit["tool_input"]["new_string"] = Value::from(floor);
    let response = &mut edit["tool_response"];
    response["oldString"] = Value::from(fixed);
    response["newString"] = Value::from(floor);
    response["originalFile"] = Value::from(OPS_FIXED);
    response["structuredPatch"][0]["lines"] = Value::from(vec![
        " ".to_owned(),
        " ".to_owned(),
        " def divide(a, b):".to_owned(),
        format!("-{fixed}"),
        format!("+{floor}"),
    ]);
    Ok(edit)
}

/// The captured run of tests `name`, which Claude Code sent in a session of its own, sent as the
/// session's: its session, prompt and transcript are the captured prompt's.
pub fn tests_run(root: &Path, name: &str) -> TestResult<Value> {
    let mut run = captured(name, root)?;
    let prompt = captured("user-prompt-submit.json", root)?;
    for field in ["session_id", "transcript_path", "prompt_id"] {
        run[field] = prompt[field].clone();
    }
    Ok(run)
}

/// The captured prompt with words of its own.
pub fn prompt(root: &Path, words: &str) -> TestResult<Value> {
    let mut prompt = captured("user-prompt-submit.json", root)?;
    prompt["prompt"] = Value::from(words);
    Ok(prompt)
}
