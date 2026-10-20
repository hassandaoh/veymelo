//! `veymelo`: Veymelo from the shell.
//!
//! - `veymelo mcp [--open FILE]` serves MCP on standard input and output with a session of its own.
//! - `veymelo mcp --bridge [CHANNEL]` serves MCP for the open app, so changes show in it live.
//! - `veymelo exec FILE COMMAND [PARAMS] [--save]` runs one command on a project file.
//! - `veymelo render FILE OUT.png [--comp C] [--time T] [--scale S]` draws a frame.
//! - `veymelo commands [WORD]` and `veymelo describe COMMAND` tell what there is.

use std::io::{self, BufReader};
use std::path::PathBuf;
use std::process::ExitCode;

use veymelo_engine::{Session, project::Project};
use veymelo_json::Value as Json;

const USAGE: &str = "\
usage:
  veymelo mcp [--open FILE]           MCP on standard input and output, a session of its own
  veymelo mcp --bridge [CHANNEL]      MCP for the open Veymelo app (changes show live)
  veymelo exec FILE COMMAND [PARAMS] [--save]
                                     run a command on a .veymelo file (PARAMS is a JSON object;
                                     the file must exist unless COMMAND is the first to make it)
  veymelo render FILE OUT.png [--comp C] [--time SECONDS] [--scale S]
                                     draw a frame of a composition as a PNG
  veymelo commands [WORD]             list the commands (those with WORD in them)
  veymelo describe COMMAND            what a command does and takes";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("mcp") => mcp(&args[1..]),
        Some("exec") => exec(&args[1..]),
        Some("render") => render(&args[1..]),
        Some("commands") => look("command.list", args.get(1).map(|w| Json::object([("search", w.as_str())])).unwrap_or(Json::Null)),
        Some("describe") => match args.get(1) {
            Some(c) => look("command.describe", Json::object([("command", c.as_str())])),
            None => Err(Failure::Usage("describe what? (veymelo describe layer.add)".into())),
        },
        Some("help" | "-h" | "--help") | None => {
            println!("{USAGE}");
            Ok(())
        }
        Some(other) => Err(Failure::Usage(format!("there is no `veymelo {other}`"))),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(Failure::Usage(why)) => {
            eprintln!("{why}\n\n{USAGE}");
            ExitCode::from(2)
        }
        Err(Failure::Command(why)) => {
            eprintln!("{why}");
            ExitCode::from(1)
        }
    }
}

enum Failure {
    /// The command line itself is wrong.
    Usage(String),
    /// What was asked for failed.
    Command(String),
}

/// The value after `--name`, and the arguments without both.
fn take_option(args: &mut Vec<String>, name: &str) -> Result<Option<String>, Failure> {
    let Some(i) = args.iter().position(|a| a == name) else { return Ok(None) };
    if i + 1 >= args.len() || args[i + 1].starts_with("--") {
        return Err(Failure::Usage(format!("{name} needs a value")));
    }
    let v = args.remove(i + 1);
    args.remove(i);
    Ok(Some(v))
}

fn take_flag(args: &mut Vec<String>, name: &str) -> bool {
    match args.iter().position(|a| a == name) {
        Some(i) => {
            args.remove(i);
            true
        }
        None => false,
    }
}

fn mcp(rest: &[String]) -> Result<(), Failure> {
    let mut args = rest.to_vec();
    let stdin = io::stdin();
    if take_flag(&mut args, "--bridge") {
        let file = args.first().map(PathBuf::from).unwrap_or_else(veymelo_mcp::default_file);
        let mut bridge = veymelo_mcp::Bridge::new(file);
        return veymelo_mcp::serve(&mut bridge, BufReader::new(stdin.lock()), io::stdout().lock()).map_err(|e| Failure::Command(e.to_string()));
    }
    let session = match take_option(&mut args, "--open")? {
        Some(path) => open(&path)?,
        None => Session::new(),
    };
    if let Some(extra) = args.first() {
        return Err(Failure::Usage(format!("`veymelo mcp` does not take {extra}")));
    }
    let mut local = veymelo_mcp::Local::new(session);
    veymelo_mcp::serve(&mut local, BufReader::new(stdin.lock()), io::stdout().lock()).map_err(|e| Failure::Command(e.to_string()))
}

fn open(path: &str) -> Result<Session, Failure> {
    Session::open(&PathBuf::from(path)).map_err(Failure::Command)
}

fn params(text: Option<&String>) -> Result<Json, Failure> {
    match text {
        None => Ok(Json::Null),
        Some(t) => Json::parse(t).map_err(|e| Failure::Usage(format!("the parameters are not JSON: {e}"))),
    }
}

fn exec(rest: &[String]) -> Result<(), Failure> {
    let mut args = rest.to_vec();
    let save = take_flag(&mut args, "--save");
    let (Some(file), Some(command)) = (args.first(), args.get(1)) else { return Err(Failure::Usage("exec needs a file and a command".into())) };
    if args.len() > 3 {
        return Err(Failure::Usage(format!("exec does not take {}", args[3])));
    }
    let p = params(args.get(2))?;
    let path = PathBuf::from(file);
    let mut s = if path.exists() { open(file)? } else { Session::with(Project::new(), Some(path.clone())) };
    let out = s.run(command, &p).map_err(|e| Failure::Command(e.message))?;
    println!("{}", out.to_pretty().trim_end());
    // a new file is written even when nothing in it changed (project.new makes it)
    if save && (s.modified() || !path.exists()) {
        s.run("project.save", &Json::object([("path", file.as_str())])).map_err(|e| Failure::Command(e.message))?;
    } else if !save && s.modified() {
        eprintln!("(not saved: add --save to keep the change)");
    }
    Ok(())
}

fn render(rest: &[String]) -> Result<(), Failure> {
    let mut args = rest.to_vec();
    let mut p = veymelo_json::Map::new();
    if let Some(c) = take_option(&mut args, "--comp")? {
        p.insert("comp", c.parse::<u64>().map(|id| Json::Number(id as f64)).unwrap_or(Json::from(c)));
    }
    for (flag, key) in [("--time", "time"), ("--scale", "scale")] {
        if let Some(v) = take_option(&mut args, flag)? {
            p.insert(key, v.parse::<f64>().map_err(|_| Failure::Usage(format!("{flag} is a number")))?);
        }
    }
    let (Some(file), Some(out)) = (args.first(), args.get(1)) else { return Err(Failure::Usage("render needs a file and where to write the picture".into())) };
    p.insert("path", out.as_str());
    let mut s = open(file)?;
    let r = s.run("render.frame", &Json::Object(p)).map_err(|e| Failure::Command(e.message))?;
    println!("{}", r.to_pretty().trim_end());
    Ok(())
}

fn look(command: &str, p: Json) -> Result<(), Failure> {
    let out = Session::new().run(command, &p).map_err(|e| Failure::Command(e.message))?;
    match out.as_array() {
        // a list of commands reads best one a line
        Some(list) if command == "command.list" => {
            for c in list {
                println!("{:<18} {}", c.get("id").and_then(Json::as_str).unwrap_or(""), c.get("summary").and_then(Json::as_str).unwrap_or(""));
            }
        }
        _ => println!("{}", out.to_pretty().trim_end()),
    }
    Ok(())
}
