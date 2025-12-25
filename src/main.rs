use rustyline::completion::{Completer, Pair};
use rustyline::config::CompletionType;
use rustyline::error::ReadlineError;
use rustyline::highlight::Highlighter;
use rustyline::hint::Hinter;
use rustyline::validate::Validator;
use rustyline::{Config, Context, Editor, Helper, Result};
#[allow(unused_imports)]
use std::env;
use std::fs;
use std::fs::OpenOptions;
use std::io::{self, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::process::Command;

fn find_executable_in_path(program_name: &str) -> Option<std::path::PathBuf> {
    let key = "PATH";
    match env::var_os(key) {
        Some(paths) => {
            for path in env::split_paths(&paths) {
                let program_path = path.join(program_name.trim());
                let my_mode = 0o111;
                match fs::metadata(&program_path) {
                    Ok(attr) => {
                        let permissions = attr.permissions();
                        if permissions.mode() & my_mode != 0 {
                            return Some(program_path);
                        }
                    }
                    Err(_) => (),
                };
            }
            None
        }
        None => None,
    }
}

fn parse_command_line(input: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current_arg = String::new();
    let mut quote_state: Option<char> = None;
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '\\' => {
                if quote_state.is_none() {
                    if let Some(next_ch) = chars.next() {
                        current_arg.push(next_ch);
                    }
                } else if quote_state == Some('\"') {
                    if let Some(next_ch) = chars.next() {
                        if next_ch == '\"' || next_ch == '\\' {
                            current_arg.push(next_ch);
                        } else {
                            current_arg.push(ch);
                            current_arg.push(next_ch);
                        }
                    } else {
                        current_arg.push(ch);
                    }
                } else {
                    current_arg.push(ch);
                }
            }
            '\"' => {
                if quote_state == Some('\"') {
                    quote_state = None;
                } else if quote_state.is_none() {
                    quote_state = Some('\"');
                } else {
                    current_arg.push(ch);
                }
            }
            '\'' => {
                if quote_state == Some('\'') {
                    quote_state = None;
                } else if quote_state.is_none() {
                    quote_state = Some('\'');
                } else {
                    current_arg.push(ch);
                }
            }
            ' ' | '\t' => {
                if quote_state.is_some() {
                    current_arg.push(ch);
                } else if !current_arg.is_empty() {
                    args.push(current_arg);
                    current_arg = String::new();
                }
            }
            _ => {
                current_arg.push(ch);
            }
        }
    }

    if !current_arg.is_empty() {
        args.push(current_arg);
    }

    args
}

struct ShellCompleter;

impl Completer for ShellCompleter {
    type Candidate = Pair;

    fn complete(
        &self,
        line: &str,
        pos: usize,
        _ctx: &Context<'_>,
    ) -> Result<(usize, Vec<Pair>)> {
        let mut candidates = Vec::new();

        let start = line[..pos].rfind(|c: char| c.is_whitespace()).map(|i| i + 1).unwrap_or(0);
        let word = &line[start..pos];

        let before_word = &line[..start];
        let is_command_pos = before_word.trim().is_empty() 
            || before_word.trim_end().ends_with('|')
            || before_word.trim_end().ends_with(';');

        if is_command_pos {
            let builtins = ["echo", "exit", "type", "pwd", "cd"];
            for builtin in &builtins {
                if builtin.starts_with(word) {
                    candidates.push(Pair {
                        display: builtin.to_string(),
                        replacement: format!("{} ", builtin),
                    });
                }
            }

            if let Some(paths) = env::var_os("PATH") {
                for path in env::split_paths(&paths) {
                    if let Ok(entries) = fs::read_dir(path) {
                        for entry in entries.flatten() {
                            if let Ok(file_name) = entry.file_name().into_string() {
                                if file_name.starts_with(word) {
                                    if let Ok(metadata) = entry.metadata() {
                                        let permissions = metadata.permissions();
                                        if permissions.mode() & 0o111 != 0 {
                                            if !candidates.iter().any(|c| c.display == file_name) {
                                                candidates.push(Pair {
                                                    display: file_name.clone(),
                                                    replacement: format!("{} ", file_name),
                                                });
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        } else {
            let (dir_path, file_prefix) = if word.contains('/') {
                let path = std::path::Path::new(word);
                if let Some(parent) = path.parent() {
                    let parent_str = if parent.as_os_str().is_empty() {
                        "./"
                    } else {
                        parent.to_str().unwrap_or("./")
                    };
                    let file_name = path.file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("");
                    (parent_str.to_string(), file_name.to_string())
                } else {
                    ("./".to_string(), word.to_string())
                }
            } else {
                ("./".to_string(), word.to_string())
            };

            if let Ok(entries) = fs::read_dir(&dir_path) {
                for entry in entries.flatten() {
                    if let Ok(file_name) = entry.file_name().into_string() {
                        if file_name.starts_with(&file_prefix) && !file_name.starts_with('.') {
                            let is_dir = entry.path().is_dir();
                            let full_path = if word.contains('/') {
                                if dir_path == "./" {
                                    file_name.clone()
                                } else {
                                    format!("{}/{}", dir_path.trim_end_matches('/'), file_name)
                                }
                            } else {
                                file_name.clone()
                            };
                            
                            let display = if is_dir {
                                format!("{}/", file_name)
                            } else {
                                file_name.clone()
                            };

                            let replacement = if is_dir {
                                format!("{}/", full_path)
                            } else {
                                format!("{} ", full_path)
                            };

                            candidates.push(Pair {
                                display,
                                replacement,
                            });
                        }
                    }
                }
            }
        }

        candidates.sort_by(|a, b| a.display.cmp(&b.display));
        Ok((start, candidates))
    }
}

impl Hinter for ShellCompleter {
    type Hint = String;

    fn hint(&self, _line: &str, _pos: usize, _ctx: &Context<'_>) -> Option<String> {
        None
    }
}

impl Highlighter for ShellCompleter {}

impl Validator for ShellCompleter {}

impl Helper for ShellCompleter {}

fn main() -> Result<()> {
    let config = Config::builder()
        .auto_add_history(true)
        .completion_type(CompletionType::Circular)
        .build();
    
    let helper = ShellCompleter;
    let mut rl: Editor<ShellCompleter, rustyline::history::DefaultHistory> = Editor::with_config(config)?;
    rl.set_helper(Some(helper));

    loop {
        io::stdout().flush().unwrap();

        let readline = rl.readline("$ ");
        match readline {
            Ok(line) => {
                let trimmed_user_input = line.trim();
                let parsed_args = parse_command_line(trimmed_user_input);
                if parsed_args.is_empty() {
                    continue;
                }
                let command = &parsed_args[0];
                let args: Vec<&str> = parsed_args[1..].iter().map(|s| s.as_str()).collect();
                match command.as_str() {
                    "type" => {
                        if args.is_empty() {
                            continue;
                        }
                        let type_item = args[0];
                        match type_item {
                            "echo" | "exit" | "type" | "pwd" | "cd" => {
                                println!("{type_item} is a shell builtin")
                            }
                            _ => match find_executable_in_path(type_item) {
                                Some(path) => println!("{type_item} is {}", path.display()),
                                None => println!("{type_item}: not found"),
                            },
                        }
                    }
                    "pwd" => {
                        println!("{}", env::current_dir().unwrap().display());
                    }
                    "cd" => {
                        if args.is_empty() {
                            continue;
                        }
                        let cd_item = args[0];
                        match cd_item {
                            ".." => {
                                let mut current = env::current_dir().unwrap();
                                current.pop();
                                env::set_current_dir(current).unwrap();
                            }
                            "~" => {
                                let home_dir = env::var("HOME").unwrap();
                                env::set_current_dir(home_dir).unwrap();
                            }
                            _ => {
                                let new_path = env::current_dir().unwrap().join(cd_item);
                                if new_path.is_dir() {
                                    env::set_current_dir(new_path).unwrap();
                                } else {
                                    eprintln!("cd: {}: No such file or directory", cd_item);
                                }
                            }
                        }
                    }
                    "exit" => {
                        break;
                    }
                    _ => match find_executable_in_path(command) {
                        Some(program_path) => {
                            let mut cmd = Command::new(&program_path);
                            cmd.arg0(command);

                            let mut redirect_pos = None;
                            for (i, arg) in args.iter().enumerate() {
                                if *arg == ">"
                                    || *arg == "1>"
                                    || *arg == "1>>"
                                    || *arg == "2>"
                                    || *arg == "2>>"
                                    || *arg == ">>"
                                {
                                    if i + 1 < args.len() {
                                        redirect_pos = Some((i, args[i + 1], arg));
                                        break;
                                    }
                                }
                            }

                            let cmd_args = if let Some((pos, _, _)) = redirect_pos {
                                &args[0..pos]
                            } else {
                                &args
                            };

                            cmd.args(cmd_args);

                            if let Some((_, filename, redirect_type)) = redirect_pos {
                                let is_append = *redirect_type == ">>"
                                    || *redirect_type == "1>>"
                                    || *redirect_type == "2>>";
                                match OpenOptions::new()
                                    .write(true)
                                    .append(is_append)
                                    .truncate(!is_append)
                                    .create(true)
                                    .open(filename)
                                {
                                    Ok(file) => {
                                        if matches!(*redirect_type, "1>" | ">" | ">>" | "1>>") {
                                            cmd.stdout(file);
                                        } else {
                                            cmd.stderr(file);
                                        }
                                    }
                                    Err(e) => {
                                        eprintln!("Error creating file {}: {}", filename, e);
                                        continue;
                                    }
                                }
                            }

                            match cmd.status() {
                                Ok(_status) => {}
                                Err(e) => {
                                    eprintln!("Error executing {}: {}", command, e);
                                }
                            }
                        }
                        None => {
                            eprintln!("{}: command not found", command);
                        }
                    },
                }
            }
            Err(ReadlineError::Interrupted) => {
                println!("^C");
                continue;
            }
            Err(ReadlineError::Eof) => {
                println!("^D");
                break;
            }
            Err(e) => {
                eprintln!("Error: {}", e);
                break;
            }
        }
    }
    Ok(())
}