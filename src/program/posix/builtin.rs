//! Commands that are run from the shell directly, without forking another
//! process.
//!
//! These commands take precedence over any executables with the same name
//! in the `$PATH`.
use std::{
    env::{self, set_var},
    io::Read,
    fs::File,
    process,
    ffi::CString,
};
use nix::{
    unistd::{chdir, Pid},
    sys::wait::WaitStatus,
};
use crate::{
    program::{Result, Error, Runtime, parse_and_run},
    process::Wait as WaitTrait,
};

/// A builtin is a custom shell command, often changing the state of the
/// shell in some way.
pub trait Builtin {
    /// Execute the shell builtin command, returning a retult of the
    /// completion.
    fn run(self, argv: Vec<CString>, runtime: &mut Runtime) -> Result<WaitStatus>;
}

/// Exit builtin, alternative to ctrl-d.
pub struct Exit;

impl Builtin for Exit {
    fn run(self, argv: Vec<CString>, runtime: &mut Runtime) -> Result<WaitStatus> {
        if argv.len() == 1 || argv.len() == 2 {
            if let Some(rl) = runtime.rl.as_mut() {
                rl.save_history(&runtime.history_path).unwrap();
            }
        }

        match argv.len() {
            0 => {
                panic!("command name not passed in argv[0]");
            },
            1 => {
                process::exit(0)
            },
            2 => {
                if let Ok(n) = str::parse(argv[1].to_str().unwrap()) {
                    process::exit(n)
                } else {
                    process::exit(2)
                }
            },
            _ => {
                eprintln!("too many arguments");
                Ok(WaitStatus::Exited(Pid::this(), 1))
            }
        }
    }
}


/// Execute commands from `file` in the current environment
///
/// TODO:
/// If file does not contain a `/`, the shell shall use the search path
/// specified by `PATH` to find the directory containing file. Unlike normal
/// command search, however, the file searched for by the `.` utility need not
/// be executable. If no readable file is found, a non-interactive shell shall
/// abort; an interactive shell shall write a diagnostic message to standard
/// error, but this condition shall not be considered a syntax error.
pub struct Dot;

impl Builtin for Dot {
    fn run(self, argv: Vec<CString>, runtime: &mut Runtime) -> Result<WaitStatus> {
        match argv.len() {
            0 => unreachable!(),
            1 => {
                eprintln!("filename argument required");
                Ok(WaitStatus::Exited(Pid::this(), 2))
            }
            2 => {
                let path = argv[1].to_str().unwrap();
                if let Ok(mut file) = File::open(&path) {
                    let mut contents = String::new();
                    if file.read_to_string(&mut contents).is_ok() {
                        parse_and_run(&contents, runtime)
                    } else {
                        Ok(WaitStatus::Exited(Pid::this(), 1))
                    }
                } else {
                    Ok(WaitStatus::Exited(Pid::this(), 1))
                }
            },
            _ => unreachable!(),

        }
    }
}

/// Wait builtin, used to block for all background jobs.
pub struct Wait;

impl Builtin for Wait {
    fn run(self, argv: Vec<CString>, runtime: &mut Runtime) -> Result<WaitStatus> {
        match argv.len() {
            0 => unreachable!(),
            1 => {
                for job in runtime.jobs.borrow().iter() {
                    job.1.leader().wait();
                }
                Ok(WaitStatus::Exited(Pid::this(), 0))
            }
            n => {
                let pid: i32 = argv[1].to_string_lossy().parse().unwrap();
                dbg!(pid);
                dbg!(&runtime.jobs);
                if let Some((id, pg)) = runtime.jobs.borrow().iter().find(|(id, pg)| {
                    pid == pg.leader().pid().as_raw()
                }) {
                    pg.leader().wait().map_err(|_| Error::Runtime)
                } else {
                    Ok(WaitStatus::Exited(Pid::this(), 1337))
                }
            },
        }
    }
}

/// Export builtin, used to set global variables.
pub struct Export;

impl Builtin for Export {
    fn run(self, argv: Vec<CString>, _: &mut Runtime) -> Result<WaitStatus> {
        match argv.len() {
            0 => unreachable!(),
            1 => {
                // TODO: Print all env vars.
                unimplemented!();
            }
            n => {
                for assignment in argv[1..n].iter() {
                    let mut split = assignment.to_str().unwrap().splitn(2, '=');
                    if let (Some(key), Some(value)) = (split.next(), split.next()) {
                        env::set_var(key, value);
                    }
                }
                Ok(WaitStatus::Exited(Pid::this(), 0))
            },
        }
    }
}

/// Change directory (`cd`) builtin.
pub struct Cd;

impl Builtin for Cd {
    fn run(self, argv: Vec<CString>, _: &mut Runtime) -> Result<WaitStatus> {
        match argv.len() {
            0 => {
                panic!("command name not passed in argv[0]");
            },
            1 => {
                let home = match env::var("HOME") {
                    Ok(path) => path,
                    Err(_) => return Err(Error::Runtime),
                };
                let dst = home.as_str();
                chdir(dst).map(|_| {
                    set_var("PWD", &dst);
                    WaitStatus::Exited(Pid::this(), 0)
                })
                          .map_err(|_| Error::Runtime)
            },
            2 => {
                let dst = argv[1].to_string_lossy();
                chdir(dst.as_ref()).map(|_| {
                        set_var("PWD", dst.as_ref());
                        WaitStatus::Exited(Pid::this(), 0)
                    })
                    .map_err(|_| Error::Runtime)
            },
            _ => {
                eprintln!("too many arguments");
                Ok(WaitStatus::Exited(Pid::this(), 1))
            }
        }
    }
}

/// Noop builtin, same idea as `true`.
pub struct Return(pub i32);

impl Builtin for Return {
    fn run(self, _: Vec<CString>, _: &mut Runtime) -> Result<WaitStatus> {
        Ok(WaitStatus::Exited(Pid::this(), self.0))
    }
}

/// Command builtin, I have no idea why you'd want this honestly.
pub struct Command;

impl Builtin for Command {
    fn run(self, argv: Vec<CString>, runtime: &mut Runtime) -> Result<WaitStatus> {
        let text = argv[1..].iter().map(|c| {
            c.to_str().unwrap()
        }).collect::<Vec<_>>().join(" ");
        parse_and_run(&text, runtime)
    }
}

/// Background job information.
pub struct Jobs;

impl Builtin for Jobs {
    fn run(self, _: Vec<CString>, runtime: &mut Runtime) -> Result<WaitStatus> {
        for (id, job) in runtime.jobs.borrow().iter() {
            println!("[{}]\t{}\t\t{}",
                     id, job.leader().pid(), job.leader().body());
        }
        Ok(WaitStatus::Exited(Pid::this(), 0))
    }
}
