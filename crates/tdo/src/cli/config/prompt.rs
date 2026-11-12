//! Asking questions on the terminal. Prompts go to stderr so stdout stays for reports.

use std::io::{self, BufRead, IsTerminal, Write};

/// Where `tdo config init` gets its answers. `None` from a question means end of input
/// (EOF or Ctrl-C): the caller stops and writes nothing.
pub trait Prompter {
    /// Whether answers can be typed (stdin is a terminal).
    fn interactive(&self) -> bool;
    /// Ask for a line. The returned text is trimmed and may be empty (Enter).
    fn line(&mut self, question: &str, default: Option<&str>) -> io::Result<Option<String>>;
    /// Ask a yes/no question; Enter takes `default`.
    fn confirm(&mut self, question: &str, default: bool) -> io::Result<Option<bool>>;
}

/// Reads stdin, prompts on stderr.
pub struct TerminalPrompter;

fn read_line() -> io::Result<Option<String>> {
    let mut s = String::new();
    if io::stdin().lock().read_line(&mut s)? == 0 {
        return Ok(None);
    }
    Ok(Some(s.trim().to_string()))
}

impl Prompter for TerminalPrompter {
    fn interactive(&self) -> bool {
        io::stdin().is_terminal()
    }

    fn line(&mut self, question: &str, default: Option<&str>) -> io::Result<Option<String>> {
        let mut err = io::stderr().lock();
        match default {
            Some(d) => write!(err, "{question} [{d}]: ")?,
            None => write!(err, "{question}: ")?,
        }
        err.flush()?;
        drop(err);
        read_line()
    }

    fn confirm(&mut self, question: &str, default: bool) -> io::Result<Option<bool>> {
        let hint = if default { "Y/n" } else { "y/N" };
        loop {
            let mut err = io::stderr().lock();
            write!(err, "{question} [{hint}]: ")?;
            err.flush()?;
            drop(err);
            let Some(a) = read_line()? else {
                return Ok(None);
            };
            match a.to_lowercase().as_str() {
                "" => return Ok(Some(default)),
                "y" | "yes" => return Ok(Some(true)),
                "n" | "no" => return Ok(Some(false)),
                _ => eprintln!("please answer y or n"),
            }
        }
    }
}
