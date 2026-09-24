use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::sync::Mutex;
use std::time::Duration;

use serde_json::Value;

/// One test binary and what running it said.
struct Ran {
    name: String,
    passed: bool,
    said: String,
    counts: Option<(u64, u64, u64)>,
    retried_after: Option<String>,
}

/// Runs the integration suite in headless Chrome, several binaries at a time.
///
/// `cargo test` runs one test binary after another, and in a page each costs a
/// browser of its own; this builds them all, then hands them to `--jobs`
/// runners at once. `--shard i/n` takes every `n`-th binary starting at the
/// `i`-th, so `n` machines share one suite.
pub fn browser(args: &[String]) -> ExitCode {
    let jobs = match value_of(args, "--jobs") {
        Some(jobs) => match jobs.parse::<usize>() {
            Ok(jobs) if jobs > 0 => jobs,
            _ => {
                eprintln!("--jobs takes a number above zero, not {jobs}");
                return ExitCode::FAILURE;
            }
        },
        None => default_jobs(),
    };

    let shard = match value_of(args, "--shard").map(shard_of) {
        Some(Ok(shard)) => Some(shard),
        Some(Err(said)) => {
            eprintln!("{said}");
            return ExitCode::FAILURE;
        }
        None => None,
    };

    let binaries = match built() {
        Ok(binaries) => binaries,
        Err(said) => {
            eprintln!("{said}");
            return ExitCode::FAILURE;
        }
    };

    let mine: Vec<PathBuf> = match shard {
        Some((at, of)) => binaries
            .into_iter()
            .enumerate()
            .filter(|(index, _)| index % of == at - 1)
            .map(|(_, binary)| binary)
            .collect(),
        None => binaries,
    };

    eprintln!("running {} test binaries, {jobs} at a time", mine.len());

    let ran = run_all(mine, jobs);
    report(&ran)
}

fn value_of<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    args.iter()
        .position(|arg| arg == flag)
        .and_then(|at| args.get(at + 1))
        .map(String::as_str)
}

fn shard_of(spelled: &str) -> Result<(usize, usize), String> {
    let refused = || format!("--shard takes i/n with 1 <= i <= n, not {spelled}");
    let (at, of) = spelled.split_once('/').ok_or_else(refused)?;
    let at: usize = at.parse().map_err(|_| refused())?;
    let of: usize = of.parse().map_err(|_| refused())?;

    match (1..=of).contains(&at) {
        true => Ok((at, of)),
        false => Err(refused()),
    }
}

/// A third of the cores: a page is a browser, and past that they queue on
/// one another rather than run.
fn default_jobs() -> usize {
    std::thread::available_parallelism()
        .map(|cores| (cores.get() / 3).max(1))
        .unwrap_or(1)
}

/// Every integration test binary of the crate, built for the page, in name
/// order so a shard is the same set on every machine.
fn built() -> Result<Vec<PathBuf>, String> {
    let output = Command::new("cargo")
        .args([
            "test",
            "-p",
            "amethystate",
            "--target",
            "wasm32-unknown-unknown",
            "--tests",
            "--no-run",
            "--message-format=json-render-diagnostics",
        ])
        .stderr(Stdio::inherit())
        .output()
        .map_err(|why| format!("cargo would not start: {why}"))?;

    if !output.status.success() {
        return Err("the test binaries did not build".into());
    }

    let mut binaries: Vec<(String, PathBuf)> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|message| message["reason"] == "compiler-artifact")
        .filter(|message| {
            message["target"]["kind"]
                .as_array()
                .is_some_and(|kinds| kinds.iter().any(|kind| kind == "test"))
        })
        .filter_map(|message| {
            let name = message["target"]["name"].as_str()?.to_string();
            let binary = PathBuf::from(message["executable"].as_str()?);
            Some((name, binary))
        })
        .collect();

    binaries.sort();
    Ok(binaries.into_iter().map(|(_, binary)| binary).collect())
}

fn run_all(binaries: Vec<PathBuf>, jobs: usize) -> Vec<Ran> {
    let queue = Mutex::new(VecDeque::from(binaries));
    let ran = Mutex::new(Vec::new());

    std::thread::scope(|scope| {
        for _ in 0..jobs {
            scope.spawn(|| {
                loop {
                    let Some(binary) = queue.lock().unwrap().pop_front() else {
                        return;
                    };
                    let one = run_one(&binary);
                    eprintln!(
                        "{} {}",
                        match one.passed {
                            true => "ok    ",
                            false => "FAILED",
                        },
                        one.name
                    );
                    ran.lock().unwrap().push(one);
                }
            });
        }
    });

    let mut ran = ran.into_inner().unwrap();
    ran.sort_by(|a, b| a.name.cmp(&b.name));
    ran
}

/// Runs a binary, and once more where the runner failed without reporting a
/// result: a failing test reports one, so a run that did not is the runner's
/// own failure, and the first one is kept for the report.
fn run_one(binary: &PathBuf) -> Ran {
    let first = attempt(binary, 1);
    if first.passed || first.counts.is_some() {
        return first;
    }

    Ran {
        retried_after: Some(first.said),
        ..attempt(binary, 2)
    }
}

fn profiles() -> PathBuf {
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target"))
        .join("browser-profiles");

    std::path::absolute(&target).unwrap_or(target)
}

fn capabilities(profile: &Path) -> Value {
    serde_json::json!({
        "goog:chromeOptions": {
            "args": [format!("--user-data-dir={}", profile.display())]
        }
    })
}

fn remove_profile(profile: &Path) -> Result<(), String> {
    let mut last = String::new();
    for _ in 0..40 {
        match std::fs::remove_dir_all(profile) {
            Ok(()) => return Ok(()),
            Err(why) if why.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(why) => last = why.to_string(),
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    Err(last)
}

fn attempt(binary: &PathBuf, try_number: u32) -> Ran {
    let name = binary
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default();

    let run = format!("{name}-{}-{try_number}", std::process::id());
    let profile = profiles().join(&run);
    let asked = profiles().join(format!("{run}.json"));

    let written = std::fs::create_dir_all(profiles())
        .and_then(|()| std::fs::write(&asked, capabilities(&profile).to_string()));
    if let Err(why) = written {
        return Ran {
            name,
            passed: false,
            said: format!("the Chrome profile for this run could not be set up: {why}"),
            counts: None,
            retried_after: None,
        };
    }

    let output = Command::new("wasm-bindgen-test-runner")
        .arg(binary)
        .env("WASM_BINDGEN_USE_BROWSER", "1")
        .env("WASM_BINDGEN_TEST_WEBDRIVER_JSON", &asked)
        .output();

    let _ = std::fs::remove_file(&asked);
    if let Err(why) = remove_profile(&profile) {
        eprintln!(
            "{name}: Chrome's profile at {} is still held and was left behind: {why}",
            profile.display()
        );
    }

    match output {
        Err(why) => Ran {
            name,
            passed: false,
            said: format!("wasm-bindgen-test-runner would not start: {why}"),
            counts: None,
            retried_after: None,
        },
        Ok(output) => {
            let said = format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            let counts = counted(&said);
            Ran {
                name,
                passed: output.status.success(),
                said,
                counts,
                retried_after: None,
            }
        }
    }
}

/// What the runner's `test result:` line counted: passed, failed, ignored.
fn counted(said: &str) -> Option<(u64, u64, u64)> {
    let line = said.lines().find(|line| line.starts_with("test result:"))?;
    let number = |label: &str| {
        line.split(';')
            .find(|part| part.trim_end().ends_with(label))?
            .split_whitespace()
            .rev()
            .nth(1)?
            .parse::<u64>()
            .ok()
    };

    Some((number("passed")?, number("failed")?, number("ignored")?))
}

fn report(ran: &[Ran]) -> ExitCode {
    let failed: Vec<&Ran> = ran.iter().filter(|one| !one.passed).collect();
    let retried: Vec<&Ran> = ran
        .iter()
        .filter(|one| one.retried_after.is_some())
        .collect();

    for one in &retried {
        eprintln!(
            "\n==== {}: the runner failed without a result and was run again; it said ====\n{}",
            one.name,
            one.retried_after.as_deref().unwrap_or_default()
        );
    }

    for one in &failed {
        eprintln!("\n==== {} ====\n{}", one.name, one.said);
    }

    let (passed, failing, ignored) = ran
        .iter()
        .filter_map(|one| one.counts)
        .fold((0, 0, 0), |(p, f, i), (op, of, oi)| {
            (p + op, f + of, i + oi)
        });
    let silent = ran.iter().filter(|one| one.counts.is_none()).count();

    eprintln!(
        "\n{} binaries: {passed} passed, {failing} failed, {ignored} ignored; \
         {silent} had nothing to run in a page; {} run again after the runner failed; \
         {} binaries failed",
        ran.len(),
        retried.len(),
        failed.len()
    );

    match failed.is_empty() {
        true => ExitCode::SUCCESS,
        false => ExitCode::FAILURE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shard_is_one_based_and_inside_its_count() {
        assert_eq!(shard_of("1/4"), Ok((1, 4)));
        assert_eq!(shard_of("4/4"), Ok((4, 4)));
        assert!(shard_of("0/4").is_err());
        assert!(shard_of("5/4").is_err());
        assert!(shard_of("two/4").is_err());
    }

    #[test]
    fn chrome_is_handed_the_profile_it_runs_in() {
        let profile = Path::new("target/browser-profiles/field-7-1");

        assert_eq!(
            capabilities(profile),
            serde_json::json!({
                "goog:chromeOptions": {
                    "args": ["--user-data-dir=target/browser-profiles/field-7-1"]
                }
            })
        );
    }

    #[test]
    fn the_counts_are_read_off_the_result_line() {
        let said = "running 3 tests\n\ntest result: FAILED. 2 passed; 1 failed; 0 ignored; \
                    0 filtered out; finished in 0.10s\n";

        assert_eq!(counted(said), Some((2, 1, 0)));
        assert_eq!(counted("no tests to run!\n"), None);
    }
}
