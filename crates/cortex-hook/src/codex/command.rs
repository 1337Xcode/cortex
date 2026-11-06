//! What a shell command Codex ran does, read from its first pipeline: a read of one file and its
//! lines, a search for a pattern in one place, or anything else. `cat`, `gc`, `Get-Content`, `nl`,
//! `head` and `sed -n 'A,Bp'` read; `rg`, `grep`, `git grep` and `Select-String` search. A read
//! piped to `head`, `sed -n 'A,Bp'` or `Select-Object -First N` read those lines; a read piped to
//! anything else, of a file's last lines, of several files, of a path with a glob or a variable,
//! or with a flag the tables do not name, is anything else: what the agent saw is not known, and
//! nothing is better than a wrong file or wrong lines. A search keeps its pattern and its one path
//! whatever follows it.

use cortex_core::LineRange;

use super::flags::{Flags, Style};
use super::words::stages;

/// The lines `head` prints when it is not told how many.
const HEAD_DEFAULT_LINES: u32 = 10;

/// `cat` as coreutils takes it: switches that change no line's place in the output. `-s`, which
/// drops blank lines, is not among them.
const CAT: Flags = Flags {
    style: Style::Posix,
    switches: &[
        "-n",
        "-b",
        "-A",
        "-v",
        "-E",
        "-T",
        "-e",
        "-t",
        "-u",
        "--number",
        "--show-all",
    ],
    valued: &[],
};

/// `Get-Content` (also `gc`, and `cat` in PowerShell). `-Tail`, `-ReadCount`, `-Delimiter` and
/// `-Wait` are not among them: they read other lines than the file's first ones.
const GET_CONTENT: Flags = Flags {
    style: Style::PowerShell,
    switches: &["-raw", "-force"],
    valued: &[
        "-path",
        "-literalpath",
        "-totalcount",
        "-head",
        "-first",
        "-encoding",
    ],
};

/// `nl`, which numbers a file's lines and prints them all.
const NL: Flags = Flags {
    style: Style::Posix,
    switches: &["-p"],
    valued: &["-b", "-d", "-f", "-h", "-i", "-l", "-n", "-s", "-v", "-w"],
};

/// `Select-Object` as a read is piped to it: only `-First N`.
const SELECT_OBJECT: Flags = Flags {
    style: Style::PowerShell,
    switches: &[],
    valued: &["-first"],
};

/// ripgrep's flags, but `--files` (which lists files and takes no pattern) and `-f` (patterns
/// from a file).
const RG: Flags = Flags {
    style: Style::Posix,
    switches: &[
        "-a",
        "-c",
        "-F",
        "-H",
        "-i",
        "-I",
        "-l",
        "-L",
        "-n",
        "-N",
        "-o",
        "-p",
        "-P",
        "-q",
        "-s",
        "-S",
        "-u",
        "-U",
        "-v",
        "-w",
        "-x",
        "-z",
        "-0",
        "-.",
        "--line-number",
        "--no-line-number",
        "--ignore-case",
        "--smart-case",
        "--case-sensitive",
        "--fixed-strings",
        "--word-regexp",
        "--line-regexp",
        "--files-with-matches",
        "--files-without-match",
        "--count",
        "--count-matches",
        "--hidden",
        "--no-ignore",
        "--no-ignore-vcs",
        "--no-heading",
        "--heading",
        "--with-filename",
        "--no-filename",
        "--vimgrep",
        "--multiline",
        "--multiline-dotall",
        "--pcre2",
        "--invert-match",
        "--only-matching",
        "--follow",
        "--unrestricted",
        "--null",
        "--column",
        "--no-messages",
        "--trim",
        "--text",
        "--search-zip",
        "--no-config",
    ],
    valued: &[
        "-e",
        "-g",
        "-t",
        "-T",
        "-A",
        "-B",
        "-C",
        "-m",
        "-M",
        "-j",
        "-E",
        "-r",
        "-d",
        "--regexp",
        "--glob",
        "--iglob",
        "--type",
        "--type-not",
        "--after-context",
        "--before-context",
        "--context",
        "--max-count",
        "--max-columns",
        "--threads",
        "--encoding",
        "--replace",
        "--max-depth",
        "--sort",
        "--sortr",
        "--color",
        "--colors",
        "--max-filesize",
        "--context-separator",
        "--path-separator",
        "--engine",
    ],
};

/// GNU grep's and git grep's flags, but `-f` (patterns from a file) and git grep's `-O`.
const GREP: Flags = Flags {
    style: Style::Posix,
    switches: &[
        "-a",
        "-c",
        "-E",
        "-F",
        "-G",
        "-P",
        "-H",
        "-h",
        "-i",
        "-I",
        "-l",
        "-L",
        "-n",
        "-o",
        "-q",
        "-r",
        "-R",
        "-s",
        "-v",
        "-w",
        "-x",
        "-y",
        "-z",
        "-Z",
        "-U",
        "-b",
        "-T",
        "-W",
        "-p",
        "--extended-regexp",
        "--fixed-strings",
        "--basic-regexp",
        "--perl-regexp",
        "--with-filename",
        "--no-filename",
        "--ignore-case",
        "--no-ignore-case",
        "--files-with-matches",
        "--files-without-match",
        "--line-number",
        "--only-matching",
        "--quiet",
        "--silent",
        "--recursive",
        "--dereference-recursive",
        "--no-messages",
        "--invert-match",
        "--word-regexp",
        "--line-regexp",
        "--count",
        "--null",
        "--text",
        "--byte-offset",
        "--initial-tab",
        "--function-context",
        "--show-function",
        "--untracked",
        "--no-index",
        "--cached",
        "--heading",
        "--break",
        "--full-name",
        "--all-match",
    ],
    valued: &[
        "-e",
        "-A",
        "-B",
        "-C",
        "-m",
        "-d",
        "-D",
        "--regexp",
        "--after-context",
        "--before-context",
        "--context",
        "--max-count",
        "--include",
        "--exclude",
        "--exclude-dir",
        "--color",
        "--colour",
        "--label",
        "--binary-files",
        "--devices",
        "--directories",
        "--group-separator",
        "--max-depth",
        "--threads",
    ],
};

/// `Select-String`'s parameters, but `-Context`, whose output lines start with `>` or spaces
/// before the file's name.
const SELECT_STRING: Flags = Flags {
    style: Style::PowerShell,
    switches: &[
        "-simplematch",
        "-casesensitive",
        "-notmatch",
        "-allmatches",
        "-list",
        "-quiet",
        "-raw",
        "-noemphasis",
    ],
    valued: &[
        "-pattern",
        "-path",
        "-literalpath",
        "-encoding",
        "-include",
        "-exclude",
    ],
};

/// What a shell command does.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Step<'a> {
    /// It reads the file at `path`: the lines `lines`, or the whole file when it names none.
    Read {
        /// The file, as the command names it.
        path: &'a str,
        /// The lines read, when not the whole file.
        lines: Option<LineRange>,
    },
    /// It searches for `pattern` in `path`, or in the working folder when it names none.
    Search {
        /// The pattern, as the command gives it.
        pattern: &'a str,
        /// The file or folder searched, as the command names it.
        path: Option<&'a str>,
    },
    /// Anything else.
    Other,
}

/// A stage that reads one file.
struct ReadStage<'a> {
    /// The file.
    path: &'a str,
    /// The lines read, when not the whole file.
    lines: Option<LineRange>,
    /// Whether the file comes out as one object (`Get-Content -Raw`): a stage after it that keeps
    /// the first lines keeps all of it.
    is_one_object: bool,
}

/// What `command` does.
pub(super) fn step(command: &str) -> Step<'_> {
    let Some(stages) = stages(command) else {
        return Step::Other;
    };
    let Some((first, rest)) = stages.split_first() else {
        return Step::Other;
    };
    let Some((program, args)) = first.split_first() else {
        return Step::Other;
    };
    let program = program.to_ascii_lowercase();
    let program = program.trim_end_matches(".exe");
    if let Some(search) = search_of(program, args) {
        return search;
    }
    let Some(read) = read_of(program, args) else {
        return Step::Other;
    };
    match rest {
        [] => Step::Read {
            path: read.path,
            lines: read.lines,
        },
        [kept] if read.lines.is_none() => match lines_kept(kept) {
            Some(_) if read.is_one_object => Step::Read {
                path: read.path,
                lines: None,
            },
            Some(lines) => Step::Read {
                path: read.path,
                lines: Some(lines),
            },
            None => Step::Other,
        },
        _ => Step::Other,
    }
}

/// The search `program` makes with `args`, when it is a search Cortex reads.
fn search_of<'a>(program: &str, args: &[&'a str]) -> Option<Step<'a>> {
    let (flags, args) = match program {
        "rg" => (&RG, args),
        "grep" | "egrep" | "fgrep" => (&GREP, args),
        "git" => match args.split_first() {
            Some((&"grep", rest)) => (&GREP, rest),
            _ => return None,
        },
        "select-string" | "sls" => (&SELECT_STRING, args),
        _ => return None,
    };
    let Some(read) = flags.read(args) else {
        return Some(Step::Other);
    };
    let given = read.values_of(&["-e", "--regexp", "-pattern"]);
    let (pattern, rest) = match given.as_slice() {
        [pattern] => (*pattern, read.plain.as_slice()),
        [] => match read.plain.split_first() {
            Some((pattern, rest)) => (*pattern, rest),
            None => return Some(Step::Other),
        },
        _ => return Some(Step::Other),
    };
    let named = read.values_of(&["-path", "-literalpath"]);
    let paths: Vec<&str> = named.into_iter().chain(rest.iter().copied()).collect();
    Some(match paths.as_slice() {
        [] => Step::Search {
            pattern,
            path: None,
        },
        [path] if is_literal(path) => Step::Search {
            pattern,
            path: Some(path),
        },
        _ => Step::Other,
    })
}

/// The read of one file `program` makes with `args`, when it is one Cortex reads.
fn read_of<'a>(program: &str, args: &[&'a str]) -> Option<ReadStage<'a>> {
    match program {
        "cat" => CAT
            .read(args)
            .and_then(|read| one_path(&read.plain))
            .map(|path| ReadStage {
                path,
                lines: None,
                is_one_object: false,
            })
            .or_else(|| get_content(args)),
        "gc" | "get-content" => get_content(args),
        "nl" => NL
            .read(args)
            .and_then(|read| one_path(&read.plain))
            .map(|path| ReadStage {
                path,
                lines: None,
                is_one_object: false,
            }),
        "head" => match head(args)? {
            (lines, Some(path)) => Some(ReadStage {
                path,
                lines: Some(lines),
                is_one_object: false,
            }),
            (_, None) => None,
        },
        "sed" => match sed(args)? {
            (lines, Some(path)) => Some(ReadStage {
                path,
                lines: Some(lines),
                is_one_object: false,
            }),
            (_, None) => None,
        },
        _ => None,
    }
}

/// A read by `Get-Content`: its one file, from `-Path`, `-LiteralPath` or a plain word, and its
/// first lines when `-TotalCount`, `-Head` or `-First` says how many; `-Raw` with a count is not
/// read.
fn get_content<'a>(args: &[&'a str]) -> Option<ReadStage<'a>> {
    let read = GET_CONTENT.read(args)?;
    let named = read.values_of(&["-path", "-literalpath"]);
    let paths: Vec<&str> = named
        .into_iter()
        .chain(read.plain.iter().copied())
        .collect();
    let path = one_path(&paths)?;
    let is_one_object = read.has_any(&["-raw"]);
    let lines = match read
        .values_of(&["-totalcount", "-head", "-first"])
        .as_slice()
    {
        [] => None,
        [count] if !is_one_object => Some(LineRange::new(1, number(count)?).ok()?),
        _ => return None,
    };
    Some(ReadStage {
        path,
        lines,
        is_one_object,
    })
}

/// The lines a read piped to the stage `words` keeps: `head`'s first lines, `sed -n`'s range, or
/// `Select-Object -First N`'s first lines; none for any other stage.
fn lines_kept(words: &[&str]) -> Option<LineRange> {
    let (program, args) = words.split_first()?;
    let program = program.to_ascii_lowercase();
    match program.trim_end_matches(".exe") {
        "head" => match head(args)? {
            (lines, None) => Some(lines),
            (_, Some(_)) => None,
        },
        "sed" => match sed(args)? {
            (lines, None) => Some(lines),
            (_, Some(_)) => None,
        },
        "select-object" | "select" => {
            let read = SELECT_OBJECT.read(args)?;
            match (
                read.plain.as_slice(),
                read.values_of(&["-first"]).as_slice(),
            ) {
                ([], [count]) => LineRange::new(1, number(count)?).ok(),
                _ => None,
            }
        }
        _ => None,
    }
}

/// `head`'s first lines, 10 unless `-n N`, `-nN`, `--lines=N` or `-N` says otherwise, and the
/// one file it reads, if it names one; none for any other flag or several files.
fn head<'a>(args: &[&'a str]) -> Option<(LineRange, Option<&'a str>)> {
    let mut count = HEAD_DEFAULT_LINES;
    let mut paths = Vec::new();
    let mut words = args.iter().copied();
    while let Some(word) = words.next() {
        if word == "-n" || word == "--lines" {
            count = number(words.next()?)?;
        } else if let Some(value) = word
            .strip_prefix("--lines=")
            .or_else(|| word.strip_prefix("-n"))
            .or_else(|| word.strip_prefix('-'))
        {
            count = number(value)?;
        } else {
            paths.push(word);
        }
    }
    let lines = LineRange::new(1, count).ok()?;
    match paths.as_slice() {
        [] => Some((lines, None)),
        [path] if is_literal(path) => Some((lines, Some(*path))),
        _ => None,
    }
}

/// `sed -n 'A,Bp'` or `sed -n 'Ap'`: lines A to B, and the one file it reads, if it names one.
/// Any other `sed`, a relative address (`10,+5p`) among them, changes or filters what it prints.
fn sed<'a>(args: &[&'a str]) -> Option<(LineRange, Option<&'a str>)> {
    let (quiet, script, path) = match args {
        [quiet, script] => (quiet, script, None),
        [quiet, script, path] if is_literal(path) => (quiet, script, Some(*path)),
        _ => return None,
    };
    if *quiet != "-n" {
        return None;
    }
    let lines = script.strip_suffix('p')?;
    let (start, end) = lines.split_once(',').unwrap_or((lines, lines));
    let lines = LineRange::new(number(start)?, number(end)?).ok()?;
    Some((lines, path))
}

/// The one path among `paths`, when there is exactly one and it is a literal path.
fn one_path<'a>(paths: &[&'a str]) -> Option<&'a str> {
    match paths {
        [path] if is_literal(path) => Some(path),
        _ => None,
    }
}

/// Whether `path` names one file or folder as written: no glob (`*`, `?`, `[`, `{`), no variable
/// (`$`, `%`, a backtick), no home folder (`~`), no list (`,`), and not standard input (`-`).
fn is_literal(path: &str) -> bool {
    !path.is_empty()
        && path != "-"
        && !path.starts_with('~')
        && !path.contains(['*', '?', '[', '{', '$', '%', '`', ','])
}

/// The number `text` writes in decimal digits alone: `+5` and `-5` are none.
fn number(text: &str) -> Option<u32> {
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lines `start` to `end`.
    fn lines(start: u32, end: u32) -> Option<LineRange> {
        LineRange::new(start, end).ok()
    }

    /// A whole-file read names its file and no lines, by each of the programs that read, in any
    /// case, with flags that take no value, and its path quoted or not.
    #[test]
    fn a_whole_file_read_names_its_file() {
        for command in [
            "Get-Content -Raw calc/ops.py",
            "get-content calc/ops.py",
            "Get-Content -Path calc/ops.py",
            "Get-Content -LiteralPath 'calc/ops.py' -Encoding utf8",
            "Get-Content -Path:calc/ops.py",
            "cat calc/ops.py",
            "cat -n \"calc/ops.py\"",
            "cat -Raw calc/ops.py",
            "gc calc/ops.py",
            "nl -ba calc/ops.py",
            "nl -b a calc/ops.py",
            "Get-Content -Raw calc/ops.py | Select-Object -First 5",
            "cat calc/ops.py 2>/dev/null",
        ] {
            assert_eq!(
                step(command),
                Step::Read {
                    path: "calc/ops.py",
                    lines: None
                },
                "{command}"
            );
        }
    }

    /// A read of a file's first lines or of a range names them: counted from the flags or from
    /// the stage the read is piped to.
    #[test]
    fn a_read_of_some_lines_names_them() {
        for (command, start, end) in [
            ("head calc/ops.py", 1, 10),
            ("head -n 40 calc/ops.py", 1, 40),
            ("head -n40 calc/ops.py", 1, 40),
            ("head --lines=40 calc/ops.py", 1, 40),
            ("head -25 calc/ops.py", 1, 25),
            ("sed -n '1,200p' calc/ops.py", 1, 200),
            ("sed -n 12p calc/ops.py", 12, 12),
            ("Get-Content calc/ops.py -TotalCount 30", 1, 30),
            ("Get-Content calc/ops.py -TotalCount:5", 1, 5),
            ("Get-Content calc/ops.py -First 5", 1, 5),
            ("cat calc/ops.py | head -n 20", 1, 20),
            ("Get-Content calc/ops.py | Select-Object -First 40", 1, 40),
            ("gc calc/ops.py | select -first:3", 1, 3),
            ("nl -ba calc/ops.py | sed -n '100,160p'", 100, 160),
            ("cat calc/ops.py | head.exe -n 2", 1, 2),
        ] {
            assert_eq!(
                step(command),
                Step::Read {
                    path: "calc/ops.py",
                    lines: lines(start, end)
                },
                "{command}"
            );
        }
    }

    /// A command whose read of a file is not known line by line is anything else: a file's last
    /// lines, several files, a read piped to a filter, a `sed` that edits, a relative address,
    /// zero lines, a count that is no number, a redirect, a glob, a variable, a list, the home
    /// folder, a flag that changes the lines, `Get-Content` given both `-Raw` and a count.
    #[test]
    fn a_read_not_known_line_by_line_is_anything_else() {
        for command in [
            "Get-Content calc/ops.py -Tail 20",
            "Get-Content calc/ops.py -Tail:5",
            "gc calc/ops.py -Last:20",
            "Get-Content calc/ops.py -ReadCount 0 | Select-Object -First 1",
            "Get-Content -Raw -TotalCount 5 calc/ops.py",
            "tail -n 20 calc/ops.py",
            "cat calc/ops.py main.py",
            "cat -s calc/ops.py | sed -n '10,20p'",
            "cat calc/ops.py | grep divide",
            "cat calc/ops.py | head -n 5 | tail -n 1",
            "sed -i 's/a/b/' calc/ops.py",
            "sed -n '1,20p;40,50p' calc/ops.py",
            "sed -n '10,+12p' calc/ops.py",
            "head -n 0 calc/ops.py",
            "head -n +5 calc/ops.py",
            "head -c 100 calc/ops.py",
            "Get-Content calc/ops.py -First many",
            "cat calc/ops.py > copy.py",
            "cat <calc/ops.py",
            "cat calc/ops.py>out.py",
            "cat calc/*.py",
            "cat $f",
            "cat ~/.ssh/id_rsa",
            "cat a.py,b.py",
            "cat -",
            "type calc/ops.py",
            "cat",
            "python -m pytest",
            "",
        ] {
            assert_eq!(step(command), Step::Other, "{command}");
        }
    }

    /// A search names its pattern and its one path, or none; flags that take a value are passed
    /// over with it, attached or not, and whatever follows the search in its pipeline.
    #[test]
    fn a_search_names_its_pattern_and_path() {
        for (command, pattern, path) in [
            ("git grep -n divide", "divide", None),
            (r"rg -n '\badd\s*\(' .", r"\badd\s*\(", Some(".")),
            ("rg -n -g '*.py' -C 3 divide calc", "divide", Some("calc")),
            ("rg -e divide -- calc", "divide", Some("calc")),
            ("rg --regexp=divide calc", "divide", Some("calc")),
            ("rg --sort path divide", "divide", None),
            ("rg -n divide --max-filesize 1M", "divide", None),
            ("rg -nA2 divide calc", "divide", Some("calc")),
            ("rg divide | head -n 20", "divide", None),
            ("rg -n divide 2>/dev/null", "divide", None),
            ("git grep -n divide 2>&1 | head -50", "divide", None),
            ("grep -rn \"def divide\" calc/", "def divide", Some("calc/")),
            ("grep -rn -edivide calc", "divide", Some("calc")),
            ("grep -rn --include '*.py' divide", "divide", None),
            (
                "grep -A 2 divide calc/ops.py",
                "divide",
                Some("calc/ops.py"),
            ),
            (
                "Select-String -Pattern divide -Path calc/ops.py",
                "divide",
                Some("calc/ops.py"),
            ),
            (
                "Select-String -Pattern:divide calc/ops.py",
                "divide",
                Some("calc/ops.py"),
            ),
            (
                "Select-String divide calc/ops.py",
                "divide",
                Some("calc/ops.py"),
            ),
            ("sls -pattern divide", "divide", None),
        ] {
            assert_eq!(step(command), Step::Search { pattern, path }, "{command}");
        }
    }

    /// A search Cortex cannot read for one pattern in one place is anything else: no pattern,
    /// several places, two patterns, patterns from a file, a flag the table does not name, a
    /// listing of files, `Select-String -Context`, a glob, a redirect of its output, and `git`
    /// doing anything but search.
    #[test]
    fn a_search_of_no_one_place_is_anything_else() {
        for command in [
            "rg --files",
            "rg --files calc",
            "rg divide calc tests",
            "rg -e a -e b calc",
            "rg -f pats.txt calc",
            "rg --type-add 'web:*.html' divide",
            "grep -rn",
            "Select-String -Context 1 divide calc/ops.py",
            "rg divide calc/*.py",
            "rg divide >hits.txt",
            "rg divide > hits.txt",
            "git status",
            "git",
            "Get-ChildItem -Recurse | Select-String divide",
        ] {
            assert_eq!(step(command), Step::Other, "{command}");
        }
    }

    /// Only the first command of a list counts, and a list's separators inside quotes separate
    /// nothing.
    #[test]
    fn only_a_list_s_first_command_counts() {
        let whole = Step::Read {
            path: "calc/ops.py",
            lines: None,
        };
        assert_eq!(step("cat calc/ops.py; rm -rf build"), whole);
        assert_eq!(step("cat calc/ops.py && python main.py"), whole);
        assert_eq!(step("cat calc/ops.py\npython main.py"), whole);
        assert_eq!(step("cd calc && cat ops.py"), Step::Other);
        assert_eq!(
            step("rg 'a|b; c && d' calc"),
            Step::Search {
                pattern: "a|b; c && d",
                path: Some("calc")
            }
        );
    }
}
