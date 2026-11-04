//! The bull's CLI commands:
//!
//! - `nandi name`: print the bull's name
//! - `nandi rename <NAME>` / `nandi rename --reset`: rename it
//! - Anything else that is the hidden phrase gets the bull's reply
//!   (`services::mascot::handle_phrase`): a line of text, never an action

use crate::services::mascot;
use crate::utils::nandi_config;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MascotCommand {
    Name,
    /// `None`: back to the default
    Rename(Option<String>),
    /// The words typed, which were the hidden phrase
    Phrase(String),
}

/// A mascot command from the arguments (after the program name), if any.
/// Other commands and flags are left to `cli`.
pub fn parse(args: &[String]) -> Result<Option<MascotCommand>, String> {
    match args.first().map(String::as_str) {
        Some("name") if args.len() == 1 => Ok(Some(MascotCommand::Name)),
        Some("name") => Err("name takes no arguments".to_string()),
        Some("rename") => match &args[1..] {
            [] => Err("rename needs a name, or --reset".to_string()),
            [flag] if flag == "--reset" => Ok(Some(MascotCommand::Rename(None))),
            words => Ok(Some(MascotCommand::Rename(Some(words.join(" "))))),
        },
        Some(first) if !first.starts_with('-') => {
            let typed = args.join(" ");
            // The name doesn't change whether it's the phrase
            Ok(mascot::handle_phrase(&typed, mascot::DEFAULT_NAME)
                .map(|_| MascotCommand::Phrase(typed)))
        }
        _ => Ok(None),
    }
}

/// Runs a mascot command against `~/.nandi/config.json`; the exit code
pub fn run(command: &MascotCommand) -> i32 {
    let dir = match nandi_config::nandi_dir() {
        Ok(dir) => dir,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    match output(&dir, command) {
        Ok(line) => {
            println!("{line}");
            0
        }
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

/// What a command prints, for the config in `dir`
pub fn output(dir: &std::path::Path, command: &MascotCommand) -> Result<String, String> {
    match command {
        MascotCommand::Name => Ok(mascot::read(dir).name),
        MascotCommand::Rename(name) => {
            let info = mascot::write(dir, name.as_deref())?;
            Ok(if info.is_default {
                format!("The bull is called {} again.", info.name)
            } else {
                mascot::line_renamed(&info.name)
            })
        }
        MascotCommand::Phrase(typed) => {
            let name = mascot::read(dir).name;
            mascot::handle_phrase(typed, &name)
                .map(|reply| reply.line)
                .ok_or_else(|| "Unknown command. Run with --help for usage.".to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(text: &str) -> Vec<String> {
        text.split_whitespace().map(str::to_string).collect()
    }

    #[test]
    fn parses_name_and_rename() {
        assert_eq!(parse(&args("name")).unwrap(), Some(MascotCommand::Name));
        assert_eq!(
            parse(&args("rename Big Gauri")).unwrap(),
            Some(MascotCommand::Rename(Some("Big Gauri".into())))
        );
        assert_eq!(
            parse(&args("rename --reset")).unwrap(),
            Some(MascotCommand::Rename(None))
        );
        assert!(parse(&args("rename")).is_err());
        assert!(parse(&args("name x")).is_err());
    }

    #[test]
    fn leaves_everything_else_to_the_cli() {
        for other in [
            "",
            "--headless",
            "relay --port 1",
            "verify --agent a",
            "C:/file.txt",
        ] {
            assert_eq!(parse(&args(other)).unwrap(), None, "{other}");
        }
    }

    #[test]
    fn renames_and_replies_to_the_phrase_with_words_only() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(output(dir.path(), &MascotCommand::Name).unwrap(), "Nandi");
        assert_eq!(
            output(dir.path(), &MascotCommand::Rename(Some("Gauri".into()))).unwrap(),
            "Nandi is now called Gauri."
        );
        assert_eq!(output(dir.path(), &MascotCommand::Name).unwrap(), "Gauri");

        let phrase = parse(&args("APPA yip yip!")).unwrap().unwrap();
        let line = output(dir.path(), &phrase).unwrap();
        assert!(line.ends_with("Gauri carries on."));
        // Nothing else changed: the name is all the config holds
        let config = crate::utils::nandi_config::load_from(dir.path()).unwrap();
        assert_eq!(config.extra.len(), 1);
        assert_eq!(
            output(dir.path(), &MascotCommand::Rename(None)).unwrap(),
            "The bull is called Nandi again."
        );
    }
}
