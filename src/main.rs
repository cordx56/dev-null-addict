//! The `dev-null-addict` command.

use std::ffi::OsString;
use std::io;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};
use dev_null_addict::api::{BaseUrl, KeyError, LogKind};
use dev_null_addict::init::{InitShell, Startup};
use dev_null_addict::shell::ShellCommand;
use dev_null_addict::{
    ClientOptions, DISABLE_ENV, RefreshInterval, SessionError, SessionOptions, StandaloneExit,
    ToggleKey,
};
use url::Url;

const TOGGLE_HELP: &str = "\
Key that opens and closes the timeline: ctrl+a to ctrl+z.

ctrl+h, ctrl+i and ctrl+m are not accepted: terminals send them as Backspace
(on some terminals), Tab and Enter, so those keys would toggle.

A single press opens the timeline 300 ms later. Press it twice within 300 ms
to send it to the shell instead.

The key is recognised however the terminal sends it: as a control byte, in the
kitty keyboard protocol, or with xterm's modifyOtherKeys. It is ignored inside a
bracketed paste. A program that has not turned bracketed paste on gets pasted
text as typed keys, so a pasted control character can still toggle.

Without --toggle, no shell is run: the timeline opens on its own over the
whole screen, and Esc at the top quits. --shell, --full-screen and SHELL_ARGS
need --toggle.";

const INIT_HELP: &str = "\
Print the snippet that starts dev-null-addict from a shell's startup file.

The options after SHELL are the ones dev-null-addict is started with, so
--toggle is required. The snippet does nothing if NO_DEV_NULL_ADDICT is
non-empty, the shell is not interactive, or standard input or output is not
a terminal.

  ~/.bashrc, ~/.zshrc:  eval \"$(dev-null-addict init sh --toggle ctrl+g)\"
  config.fish:          dev-null-addict init fish --toggle ctrl+g | source

For fish, the snippet starts fish itself inside unless --shell is given, since
$SHELL often names another shell for fish users.";

const AFTER_HELP: &str = "\
Start it from the end of ~/.bashrc or ~/.zshrc:

  eval \"$(dev-null-addict init sh --toggle ctrl+g)\"

or of ~/.config/fish/config.fish:

  dev-null-addict init fish --toggle ctrl+g | source

The shell inside gets NO_DEV_NULL_ADDICT=1, so the snippet does nothing there.
Set NO_DEV_NULL_ADDICT yourself to skip it for a shell.

If the wrapper can't start, it prints why and runs the shell without itself.
When the shell exits, dev-null-addict exits with its exit code.

Without --toggle, the timeline opens on its own over the whole screen, and Esc
at the top quits.

Sign in once with `dev-null-addict login`.";

#[derive(Debug, Parser)]
#[command(
    version,
    about = "/dev/null over your shell, one key away",
    after_long_help = AFTER_HELP,
    args_conflicts_with_subcommands = true,
    subcommand_negates_reqs = true
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    #[command(flatten)]
    run: RunArgs,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Approve this machine as a device of your /dev/null account
    Login(LoginArgs),
    /// Print the snippet that starts dev-null-addict from a shell's startup
    /// file
    #[command(long_about = INIT_HELP)]
    Init(InitArgs),
}

#[derive(Debug, PartialEq, Eq, Args)]
struct RunArgs {
    /// Open over the whole screen instead of in a window
    #[arg(long, requires = "toggle")]
    full_screen: bool,

    /// Key that opens and closes the timeline: ctrl+a to ctrl+z, except
    /// ctrl+h, ctrl+i and ctrl+m; without it, the timeline runs on its own
    #[arg(long, value_name = "KEY", long_help = TOGGLE_HELP)]
    toggle: Option<ToggleKey>,

    /// Shell to run [default: $SHELL, else /bin/sh]
    #[arg(long, value_name = "PATH", requires = "toggle")]
    shell: Option<PathBuf>,

    /// Log to show first: following, latest or trending
    #[arg(long, value_name = "LOG", default_value_t = LogKind::default())]
    log: LogKind,

    /// Seconds between refreshes while the timeline is open (at least 2)
    #[arg(long, value_name = "SECONDS", default_value_t = RefreshInterval::default())]
    refresh_interval: RefreshInterval,

    #[command(flatten)]
    account: AccountArgs,

    /// Arguments for the shell
    #[arg(last = true, value_name = "SHELL_ARGS", requires = "toggle")]
    shell_args: Vec<OsString>,
}

#[derive(Debug, PartialEq, Eq, Args)]
struct AccountArgs {
    /// Device key [default: ${XDG_DATA_HOME:-~/.local/share}/dev-null-addict/key]
    #[arg(long, value_name = "PATH")]
    key: Option<PathBuf>,

    /// /dev/null to talk to
    #[arg(long, value_name = "URL", default_value_t = BaseUrl::default())]
    api_url: BaseUrl,
}

#[derive(Debug, Args)]
struct InitArgs {
    /// sh (for bash, zsh and other POSIX shells) or fish
    #[arg(id = "init_shell", value_name = "SHELL", requires = "toggle")]
    shell: InitShell,

    /// What the snippet starts dev-null-addict with
    #[command(flatten)]
    run: RunArgs,
}

#[derive(Debug, Args)]
struct LoginArgs {
    #[command(flatten)]
    account: AccountArgs,

    /// Name of this device on /dev/null [default: dev-null-addict on <hostname>]
    #[arg(long, value_name = "NAME")]
    device_name: Option<String>,
}

/// Why the wrapper did not run the shell.
#[derive(Debug, thiserror::Error)]
enum StartError {
    #[error(
        "dev-null-addict was started inside itself; check {var} before starting it (see --help)",
        var = DISABLE_ENV
    )]
    Nested,
    #[error(transparent)]
    Args(clap::Error),
    #[error(transparent)]
    Key(#[from] KeyError),
    #[error("could not start the async runtime")]
    Runtime(#[source] io::Error),
    #[error(transparent)]
    Session(#[from] SessionError),
    #[error("dev-null-addict panicked")]
    Panicked,
}

/// Parses the command line and runs the command.
///
/// Without `--toggle`, runs the standalone timeline. With it, runs the wrapper
/// and falls back to the shell on failure. Parse errors fall back only when
/// `--toggle` occurs before `--`; help, version, login and init exit as clap does.
fn main() -> ExitCode {
    let argv: Vec<_> = std::env::args_os().collect();
    match Cli::try_parse_from(&argv) {
        Ok(Cli {
            command: Some(Command::Login(args)),
            ..
        }) => login(args),
        Ok(Cli {
            command: Some(Command::Init(args)),
            ..
        }) => init(args),
        Ok(cli) if cli.run.toggle.is_some() => wrap(cli.run),
        Ok(cli) => standalone(cli.run),
        Err(error) => {
            if matches!(
                error.kind(),
                clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion
            ) {
                let _ = error.print();
                ExitCode::SUCCESS
            } else if !has_toggle(&argv)
                || argv
                    .get(1)
                    .is_some_and(|arg| arg == "login" || arg == "init")
            {
                let _ = error.print();
                ExitCode::from(2)
            } else {
                fall_back(
                    &ShellCommand::from_env(std::iter::empty::<OsString>()),
                    &StartError::Args(error),
                )
            }
        }
    }
}

fn has_toggle(argv: &[OsString]) -> bool {
    argv.iter()
        .skip(1)
        .take_while(|arg| *arg != "--")
        .any(|arg| arg == "--toggle" || arg.as_encoded_bytes().starts_with(b"--toggle="))
}

fn standalone(args: RunArgs) -> ExitCode {
    let result = std::panic::catch_unwind(|| -> Result<_, StartError> {
        let options = client_options(args.account, args.log, args.refresh_interval)?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(StartError::Runtime)?;
        runtime
            .block_on(dev_null_addict::run_standalone(options))
            .map_err(StartError::from)
    })
    .unwrap_or(Err(StartError::Panicked));
    match result {
        Ok(StandaloneExit::Quit) => ExitCode::SUCCESS,
        Ok(StandaloneExit::Signal(signal)) => ExitCode::from((128 + signal) as u8),
        Err(error) => {
            eprintln!("dev-null-addict: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Runs the shell in the wrapper.
///
/// With [`DISABLE_ENV`] already set, the wrapper was started from inside
/// itself, which is [`StartError::Nested`]. Every failure, including a panic
/// (caught around the runtime), ends in [`fall_back`]. The shell exiting ends
/// the wrapper with the shell's exit code.
fn wrap(args: RunArgs) -> ExitCode {
    let shell = args.shell.as_ref().map_or_else(
        || ShellCommand::from_env(args.shell_args.clone()),
        |program| ShellCommand::new(program, args.shell_args.clone()),
    );
    let result = std::panic::catch_unwind(|| -> Result<_, StartError> {
        if std::env::var_os(DISABLE_ENV).is_some_and(|value| !value.is_empty()) {
            return Err(StartError::Nested);
        }
        let options = session_options(args)?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(StartError::Runtime)?;
        runtime
            .block_on(dev_null_addict::run(options))
            .map_err(StartError::from)
    });
    match result {
        Ok(Ok(status)) => status.into(),
        Ok(Err(error)) => fall_back(&shell, &error),
        Err(_) => fall_back(&shell, &StartError::Panicked),
    }
}

/// Prints why the wrapper did not run, and becomes [`recovery_shell`].
/// Returns only if that fails, or if there is no shell to become.
///
/// This is the recovery path for a failed wrapper start.
fn fall_back(shell: &ShellCommand, error: &StartError) -> ExitCode {
    eprintln!("dev-null-addict: {error}");
    if let Some(shell) = recovery_shell(shell) {
        eprintln!(
            "dev-null-addict: could not start fallback shell: {}",
            shell.exec()
        );
    } else {
        eprintln!(
            "dev-null-addict: cannot safely skip startup files for {}",
            shell.program().display()
        );
    }
    ExitCode::FAILURE
}

/// The shell to fall back to.
///
/// It is `shell` itself, unless [`DISABLE_ENV`] is already set. Then the
/// wrapper was started from inside itself by a startup file that does not
/// check the variable, and starting the shell as usual would start the
/// wrapper again, and so on forever. So it is the shell without its startup
/// files ([`ShellCommand::without_startup_files`]), or `None` for a shell
/// that can't skip them.
fn recovery_shell(shell: &ShellCommand) -> Option<ShellCommand> {
    if std::env::var_os(DISABLE_ENV).is_some_and(|value| !value.is_empty()) {
        shell.without_startup_files()
    } else {
        Some(shell.clone())
    }
}

/// Prints the snippet for [`std::env::current_exe`] started with `args.run`,
/// as [`run_args`] writes them.
fn init(args: InitArgs) -> ExitCode {
    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
        use std::io::Write;
        let startup = Startup::new(std::env::current_exe()?, run_args(&args.run));
        let snippet = dev_null_addict::init::snippet(args.shell, &startup)?;
        io::stdout().lock().write_all(snippet.as_bytes())?;
        Ok(())
    })();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("dev-null-addict init: {error}");
            ExitCode::FAILURE
        }
    }
}

/// `args` as command-line arguments again, in a fixed order, leaving out
/// those at their default.
fn run_args(args: &RunArgs) -> Vec<OsString> {
    let mut words = Vec::new();
    let mut option = |name: &str, value: OsString| {
        // One word also preserves values beginning with a dash.
        let mut word = OsString::from(name);
        word.push("=");
        word.push(value);
        words.push(word);
    };
    if let Some(toggle) = args.toggle {
        option("--toggle", toggle.to_string().into());
    }
    if let Some(shell) = &args.shell {
        option("--shell", shell.as_os_str().to_owned());
    }
    if args.log != LogKind::default() {
        option("--log", args.log.to_string().into());
    }
    if args.refresh_interval != RefreshInterval::default() {
        option(
            "--refresh-interval",
            args.refresh_interval.to_string().into(),
        );
    }
    if let Some(key) = &args.account.key {
        option("--key", key.as_os_str().to_owned());
    }
    if args.account.api_url != BaseUrl::default() {
        option("--api-url", args.account.api_url.to_string().into());
    }
    if args.full_screen {
        words.push("--full-screen".into());
    }
    if !args.shell_args.is_empty() {
        words.push("--".into());
        words.extend(args.shell_args.iter().cloned());
    }
    words
}

/// Approves the device key, making it first if there is none.
///
/// It prints the approval page's URL and opens it in the browser, then
/// explains where to find the `Location` to paste, reads it, and redeems the
/// code. See the README for the steps in each browser.
fn login(args: LoginArgs) -> ExitCode {
    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
        use dev_null_addict::api::{Client, DeviceKey};
        use dev_null_addict::auth::Authorization;
        use std::io::Write;
        let path = args.account.key.map_or_else(DeviceKey::default_path, Ok)?;
        let name = args.device_name.unwrap_or_else(default_device_name);
        let key = match DeviceKey::load(&path) {
            Ok(key) => key,
            Err(KeyError::Missing(_)) => DeviceKey::generate(&path, &name)?,
            Err(error) => return Err(error.into()),
        };
        let authorization = Authorization::new(&args.account.api_url, &key, &name)?;
        println!("Open this approval page:\n{}\n", authorization.url());
        println!(
            "Before clicking Approve, open developer tools and select Network.\nAfter approving, find POST /mobile/authorize (303), copy its Location\nresponse header (devnull://...), and paste it below. Cancel the browser's\nrequest to open another app. See README for browser-specific shortcuts."
        );
        open_in_browser(authorization.url());
        print!("Location: ");
        io::stdout().flush()?;
        let mut pasted = String::new();
        io::stdin().read_line(&mut pasted)?;
        let approval = authorization.accept(&pasted)?;
        let client = Client::new(args.account.api_url, key)?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let user = runtime.block_on(client.approve_device(&approval))?;
        println!("Signed in as @{}.", user.username);
        Ok(())
    })();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("dev-null-addict login: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Opens `url` with `xdg-open` or, on macOS, `open`. It is fine if that does
/// not work, the URL is printed anyway.
fn open_in_browser(url: &Url) {
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    if let Ok(mut child) = std::process::Command::new(opener)
        .arg(url.as_str())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        let _ = std::thread::Builder::new()
            .name("browser-opener".to_owned())
            .spawn(move || {
                let _ = child.wait();
            });
    }
}

/// `dev-null-addict on <hostname>`.
fn default_device_name() -> String {
    format!(
        "dev-null-addict on {}",
        gethostname::gethostname().to_string_lossy()
    )
}

/// The options for [`dev_null_addict::run`], with the defaults of the arguments
/// that have none in clap filled in.
fn session_options(args: RunArgs) -> Result<SessionOptions, KeyError> {
    let shell = match args.shell {
        Some(program) => ShellCommand::new(program, args.shell_args),
        None => ShellCommand::from_env(args.shell_args),
    };
    Ok(SessionOptions {
        layout: if args.full_screen {
            dev_null_addict::OverlayLayout::FullScreen
        } else {
            dev_null_addict::OverlayLayout::Float
        },
        toggle: args.toggle.expect("clap requires --toggle for the wrapper"),
        shell,
        client: client_options(args.account, args.log, args.refresh_interval)?,
    })
}

fn client_options(
    account: AccountArgs,
    log: LogKind,
    refresh_interval: RefreshInterval,
) -> Result<ClientOptions, KeyError> {
    Ok(ClientOptions {
        key_path: account
            .key
            .map_or_else(dev_null_addict::api::DeviceKey::default_path, Ok)?,
        base_url: account.api_url,
        log,
        refresh_interval,
    })
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::Cli;

    // Guard clap relationships across standalone, wrapper and subcommand arguments.
    #[test]
    fn command_line_is_consistent() {
        Cli::command().debug_assert();
    }
    // Standalone defaults must round-trip without accidentally enabling the wrapper.
    #[test]
    fn no_arguments_select_standalone() {
        use clap::Parser;
        let cli = Cli::try_parse_from(["dev-null-addict"]).unwrap();
        assert!(cli.command.is_none());
        assert!(cli.run.toggle.is_none());
        assert!(super::run_args(&cli.run).is_empty());
    }

    // Wrapper-only arguments and init must never silently start a standalone UI.
    #[test]
    fn wrapper_arguments_require_toggle() {
        use clap::Parser;
        for args in [
            vec!["--shell", "/bin/sh"],
            vec!["--full-screen"],
            vec!["--", "args"],
            vec!["init", "sh"],
        ] {
            let error =
                Cli::try_parse_from(std::iter::once("dev-null-addict").chain(args)).unwrap_err();
            assert_eq!(
                error.kind(),
                clap::error::ErrorKind::MissingRequiredArgument
            );
            assert_eq!(error.exit_code(), 2);
            assert!(error.to_string().contains("--toggle"));
        }
    }

    // Only an explicit wrapper option before the shell boundary authorizes fallback.
    #[test]
    fn fallback_toggle_respects_argument_boundary() {
        for (args, expected) in [
            (vec!["--toggle", "ctrl+g", "--", "args"], true),
            (vec!["--", "--toggle", "ctrl+g"], false),
            (vec!["--toggle=ctrl+g"], true),
            (vec!["--", "--toggle=ctrl+g"], false),
            (vec!["--toggle-extra=ctrl+g"], false),
            (vec!["--key", "missing"], false),
        ] {
            let argv = std::iter::once("dev-null-addict")
                .chain(args)
                .map(std::ffi::OsString::from)
                .collect::<Vec<_>>();
            assert_eq!(super::has_toggle(&argv), expected, "{argv:?}");
        }
    }

    // Serializing parsed options must preserve paths and trailing arguments while omitting defaults.
    #[test]
    fn wrapper_options_round_trip() {
        use super::run_args;
        use clap::Parser;
        for words in [
            vec!["--toggle", "ctrl+g"],
            vec![
                "--toggle=ctrl+a",
                "--shell=/a path/it's",
                "--full-screen",
                "--log=trending",
                "--refresh-interval=13",
                "--key=/key path/it's",
                "--api-url=https://example.test/",
                "--",
                "-i",
                "a b",
                "it's",
                "",
                "--shell=child",
            ],
            vec![
                "--toggle=ctrl+z",
                "--log=latest",
                "--shell=-shell",
                "--key=-key",
                "--refresh-interval=2",
            ],
            vec![
                "--toggle=ctrl+g",
                "--log=following",
                "--refresh-interval=5",
                "--api-url=https://dev-null.sh",
            ],
        ] {
            let first =
                Cli::try_parse_from(std::iter::once("dev-null-addict").chain(words)).unwrap();
            let saved = run_args(&first.run);
            let second = Cli::try_parse_from(
                std::iter::once(std::ffi::OsString::from("dev-null-addict")).chain(saved.clone()),
            )
            .unwrap();
            assert_eq!(first.run, second.run);
            if first.run.shell.is_none() {
                assert_eq!(saved, vec![std::ffi::OsString::from("--toggle=ctrl+g")]);
            }
        }
    }

    // Init accepts the same wrapper options without treating them as top-level wrapper invocation.
    #[test]
    fn init_options_round_trip() {
        use super::{Command, run_args};
        use clap::Parser;
        for shell in ["sh", "fish"] {
            let cli = Cli::try_parse_from([
                "dev-null-addict",
                "init",
                shell,
                "--full-screen",
                "--toggle=ctrl+g",
                "--shell=/a path/it's",
                "--log=latest",
                "--",
                "-i",
                "a b",
            ])
            .unwrap();
            let Some(Command::Init(init)) = cli.command else {
                panic!("init command")
            };
            let second = Cli::try_parse_from(
                std::iter::once(std::ffi::OsString::from("dev-null-addict"))
                    .chain(run_args(&init.run)),
            )
            .unwrap();
            assert_eq!(init.run, second.run);
            assert_eq!(init.shell.as_str(), shell);
        }
    }
}
