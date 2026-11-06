//! A program's flags read from its words by a table of the flags it takes: which are switches and
//! which take a value, POSIX style (`-abc` clusters, a short flag's value attached or in the next
//! word, `--name=value` or `--name value`, `--` ending the flags) or PowerShell style (`-Name`,
//! `-Name value`, `-Name:value`, in any case). A flag the table does not name refuses the whole
//! command: guessing whether it takes a value could take a path for a pattern.

/// The flags one program takes.
pub(super) struct Flags {
    /// How its flags are written.
    pub(super) style: Style,
    /// The flags that take no value: short letters as `-x`, long ones as `--name`, PowerShell's in
    /// lower case.
    pub(super) switches: &'static [&'static str],
    /// The flags that take a value, written as the switches are.
    pub(super) valued: &'static [&'static str],
}

/// How a program's flags are written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Style {
    /// `-abc`, `-e value`, `-evalue`, `--name=value`, `--name value`, `--`.
    Posix,
    /// `-Name`, `-Name value`, `-Name:value`, matched in any case.
    PowerShell,
}

/// The words of a command read against its program's flags.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct Read<'a> {
    /// The words that are no flag and no flag's value, in order.
    pub(super) plain: Vec<&'a str>,
    /// Each switch given, as the table names it.
    pub(super) switches: Vec<&'static str>,
    /// Each flag given a value, as the table names it, with its value, in order.
    pub(super) values: Vec<(&'static str, &'a str)>,
}

impl<'a> Read<'a> {
    /// The values given to any of `names`, in order.
    pub(super) fn values_of(&self, names: &[&str]) -> Vec<&'a str> {
        self.values
            .iter()
            .filter(|(name, _)| names.contains(name))
            .map(|(_, value)| *value)
            .collect()
    }

    /// Whether any of `names` was given, as a switch or with a value.
    pub(super) fn has_any(&self, names: &[&str]) -> bool {
        self.switches.iter().any(|name| names.contains(name))
            || self.values.iter().any(|(name, _)| names.contains(name))
    }
}

impl Flags {
    /// `args` read against these flags; none when a flag is not in the table or a value is
    /// missing.
    pub(super) fn read<'a>(&self, args: &[&'a str]) -> Option<Read<'a>> {
        let mut read = Read::default();
        let mut words = args.iter().copied();
        while let Some(word) = words.next() {
            if word == "--" && self.style == Style::Posix {
                read.plain.extend(words.by_ref());
                break;
            }
            if !word.starts_with('-') || word.len() == 1 {
                read.plain.push(word);
                continue;
            }
            let taken = match self.style {
                Style::PowerShell => self.read_powershell(word, &mut words, &mut read),
                Style::Posix if word.starts_with("--") => {
                    self.read_long(word, &mut words, &mut read)
                }
                Style::Posix => self.read_cluster(word, &mut words, &mut read),
            };
            taken?;
        }
        Some(read)
    }

    /// Reads the PowerShell parameter `word` (`-Name`, `-Name:value`), its value from `words`
    /// when it takes one and none is attached.
    fn read_powershell<'a>(
        &self,
        word: &'a str,
        words: &mut impl Iterator<Item = &'a str>,
        read: &mut Read<'a>,
    ) -> Option<()> {
        let (name, attached) = match word.split_once(':') {
            Some((name, value)) => (name, Some(value)),
            None => (word, None),
        };
        let name = name.to_ascii_lowercase();
        if let Some(valued) = self.valued.iter().find(|valued| **valued == name) {
            let value = attached.or_else(|| words.next())?;
            read.values.push((valued, value));
            return Some(());
        }
        let switch = self.switches.iter().find(|switch| **switch == name)?;
        read.switches.push(switch);
        Some(())
    }

    /// Reads the long flag `word` (`--name`, `--name=value`), its value from `words` when it
    /// takes one and none is attached.
    fn read_long<'a>(
        &self,
        word: &'a str,
        words: &mut impl Iterator<Item = &'a str>,
        read: &mut Read<'a>,
    ) -> Option<()> {
        let (name, attached) = match word.split_once('=') {
            Some((name, value)) => (name, Some(value)),
            None => (word, None),
        };
        if let Some(valued) = self.valued.iter().find(|valued| **valued == name) {
            let value = attached.or_else(|| words.next())?;
            read.values.push((valued, value));
            return Some(());
        }
        let switch = self
            .switches
            .iter()
            .find(|switch| **switch == name)
            .filter(|_| attached.is_none())?;
        read.switches.push(switch);
        Some(())
    }

    /// Reads the cluster of short flags `word` (`-rn`, `-nA2`, `-edivide`): switches, then at
    /// most one flag that takes a value, the rest of the word or the next of `words`.
    fn read_cluster<'a>(
        &self,
        word: &'a str,
        words: &mut impl Iterator<Item = &'a str>,
        read: &mut Read<'a>,
    ) -> Option<()> {
        let letters = word.get(1..)?;
        for (at, letter) in letters.char_indices() {
            let name = format!("-{letter}");
            if let Some(valued) = self.valued.iter().find(|valued| **valued == name) {
                let rest = letters.get(at + letter.len_utf8()..).unwrap_or_default();
                let value = if rest.is_empty() { words.next()? } else { rest };
                read.values.push((valued, value));
                return Some(());
            }
            let switch = self.switches.iter().find(|switch| **switch == name)?;
            read.switches.push(switch);
        }
        Some(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A table of a few flags in each style.
    const POSIX: Flags = Flags {
        style: Style::Posix,
        switches: &["-n", "-r", "-i", "--count", "--hidden"],
        valued: &["-e", "-A", "--regexp", "--glob"],
    };

    /// A table of a few PowerShell parameters.
    const POWERSHELL: Flags = Flags {
        style: Style::PowerShell,
        switches: &["-raw"],
        valued: &["-path", "-totalcount"],
    };

    /// POSIX flags: a cluster of switches, a short flag's value attached or in the next word, a
    /// long flag's value after `=` or in the next word, the plain words in order, `--` ending
    /// the flags.
    #[test]
    fn posix_flags_are_read_in_each_form() {
        let read = POSIX.read(&[
            "-rn",
            "-edivide",
            "-A",
            "2",
            "--glob=*.py",
            "--regexp",
            "x",
            "--hidden",
            "calc",
            "--",
            "-n",
        ]);
        assert_eq!(
            read,
            Some(Read {
                plain: vec!["calc", "-n"],
                switches: vec!["-r", "-n", "--hidden"],
                values: vec![
                    ("-e", "divide"),
                    ("-A", "2"),
                    ("--glob", "*.py"),
                    ("--regexp", "x")
                ],
            })
        );
        let cluster = POSIX.read(&["-nA2", "a"]);
        assert_eq!(cluster.map(|read| read.values), Some(vec![("-A", "2")]));
    }

    /// A flag the table does not name, a value missing, and a switch given a value refuse the
    /// command.
    #[test]
    fn an_unknown_flag_or_a_missing_value_refuses_the_command() {
        for args in [
            &["--sort", "path", "divide"][..],
            &["-x", "divide"],
            &["-rz", "divide"],
            &["divide", "-e"],
            &["--count=2", "divide"],
        ] {
            assert_eq!(POSIX.read(args), None, "{args:?}");
        }
    }

    /// PowerShell parameters in any case, a value in the next word or after `:`, and an unknown
    /// parameter refused.
    #[test]
    fn powershell_parameters_are_read_in_each_form() {
        let read = POWERSHELL.read(&["-Raw", "-Path", "a.py", "-TotalCount:5"]);
        assert_eq!(
            read,
            Some(Read {
                plain: vec![],
                switches: vec!["-raw"],
                values: vec![("-path", "a.py"), ("-totalcount", "5")],
            })
        );
        assert_eq!(POWERSHELL.read(&["a.py", "-Tail", "5"]), None);
        assert_eq!(POWERSHELL.read(&["a.py", "-TotalCount"]), None);
    }
}
