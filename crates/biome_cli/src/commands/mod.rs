use crate::changed::{get_changed_files, get_staged_files};
use crate::cli_options::{cli_options, CliOptions, ColorsArg};
use crate::diagnostics::DeprecatedConfigurationFile;
use crate::execute::Stdin;
use crate::logging::LoggingKind;
use crate::{CliDiagnostic, CliSession, LoggingLevel, VERSION};
use biome_configuration::css::PartialCssLinter;
use biome_configuration::javascript::PartialJavascriptLinter;
use biome_configuration::json::PartialJsonLinter;
use biome_configuration::linter::RuleSelector;
use biome_configuration::{
    css::partial_css_formatter, css::partial_css_linter, javascript::partial_javascript_formatter,
    javascript::partial_javascript_linter, json::partial_json_formatter, json::partial_json_linter,
    partial_configuration, partial_files_configuration, partial_formatter_configuration,
    partial_linter_configuration, vcs::partial_vcs_configuration, vcs::PartialVcsConfiguration,
    PartialCssFormatter, PartialFilesConfiguration, PartialFormatterConfiguration,
    PartialJavascriptFormatter, PartialJsonFormatter, PartialLinterConfiguration,
};
use biome_configuration::{BiomeDiagnostic, PartialConfiguration};
use biome_console::{markup, Console, ConsoleExt};
use biome_diagnostics::{Diagnostic, PrintDiagnostic};
use biome_fs::{BiomePath, FileSystem};
use biome_service::configuration::LoadedConfiguration;
use biome_service::documentation::Doc;
use biome_service::workspace::{FixFileMode, OpenProjectParams, UpdateProjectParams};
use biome_service::{DynRef, WorkspaceError};
use bpaf::Bpaf;
use std::ffi::OsString;
use std::path::PathBuf;

pub(crate) mod check;
pub(crate) mod ci;
pub(crate) mod clean;
pub(crate) mod daemon;
pub(crate) mod explain;
pub(crate) mod format;
pub(crate) mod init;
pub(crate) mod lint;
pub(crate) mod migrate;
pub(crate) mod rage;
pub(crate) mod search;
pub(crate) mod version;

#[derive(Debug, Clone, Bpaf)]
#[bpaf(options, version(VERSION))]
/// Biome official CLI. Use it to check the health of your project or run it to check single files.
pub enum BiomeCommand {
    /// Shows the Biome version information and quit
    #[bpaf(command)]
    Version(#[bpaf(external(cli_options), hide_usage)] CliOptions),

    #[bpaf(command)]
    /// Prints information for debugging
    Rage(
        #[bpaf(external(cli_options), hide_usage)] CliOptions,
        /// Prints the Biome daemon server logs
        #[bpaf(long("daemon-logs"), switch)]
        bool,
        /// Prints the Biome configuration that the applied formatter configuration
        #[bpaf(long("formatter"), switch)]
        bool,
        /// Prints the Biome configuration that the applied linter configuration
        #[bpaf(long("linter"), switch)]
        bool,
    ),
    /// Start the Biome daemon server process
    #[bpaf(command)]
    Start(
        /// Allows to set a custom file path to the configuration file,
        /// or a custom directory path to find `biome.json` or `biome.jsonc`
        #[bpaf(env("BIOME_CONFIG_PATH"), long("config-path"), argument("PATH"))]
        Option<PathBuf>,
    ),

    /// Stop the Biome daemon server process
    #[bpaf(command)]
    Stop,

    /// Runs formatter, linter and import sorting to the requested files.
    #[bpaf(command)]
    Check {
        /// Writes safe fixes, formatting and import sorting
        #[bpaf(long("write"), switch)]
        write: bool,

        /// Allow to do unsafe fixes, should be used with `--write` or `--fix`
        #[bpaf(long("unsafe"), switch)]
        unsafe_: bool,

        /// Alias for `--write`, writes safe fixes, formatting and import sorting
        #[bpaf(long("fix"), switch, hide_usage)]
        fix: bool,

        /// Alias for `--write`, writes safe fixes, formatting and import sorting
        #[bpaf(long("apply"), switch, hide_usage)]
        apply: bool,

        /// Alias for `--write --unsafe`, writes safe and unsafe fixes, formatting and import sorting
        #[bpaf(long("apply-unsafe"), switch, hide_usage)]
        apply_unsafe: bool,

        /// Allow to enable or disable the formatter check.
        #[bpaf(
            long("formatter-enabled"),
            argument("true|false"),
            optional,
            hide_usage
        )]
        formatter_enabled: Option<bool>,
        /// Allow to enable or disable the linter check.
        #[bpaf(long("linter-enabled"), argument("true|false"), optional, hide_usage)]
        linter_enabled: Option<bool>,
        /// Allow to enable or disable the organize imports.
        #[bpaf(
            long("organize-imports-enabled"),
            argument("true|false"),
            optional,
            hide_usage
        )]
        organize_imports_enabled: Option<bool>,
        #[bpaf(external(partial_configuration), hide_usage, optional)]
        configuration: Option<PartialConfiguration>,
        #[bpaf(external, hide_usage)]
        cli_options: CliOptions,
        /// Use this option when you want to format code piped from `stdin`, and print the output to `stdout`.
        ///
        /// The file doesn't need to exist on disk, what matters is the extension of the file. Based on the extension, Biome knows how to check the code.
        ///
        /// Example: `echo 'let a;' | biome check --stdin-file-path=file.js`
        #[bpaf(long("stdin-file-path"), argument("PATH"), hide_usage)]
        stdin_file_path: Option<String>,

        /// When set to true, only the files that have been staged (the ones prepared to be committed)
        /// will be linted.
        #[bpaf(long("staged"), switch)]
        staged: bool,

        /// When set to true, only the files that have been changed compared to your `defaultBranch`
        /// configuration will be linted.
        #[bpaf(long("changed"), switch)]
        changed: bool,

        /// Use this to specify the base branch to compare against when you're using the --changed
        /// flag and the `defaultBranch` is not set in your biome.json
        #[bpaf(long("since"), argument("REF"))]
        since: Option<String>,

        /// Single file, single path or list of paths
        #[bpaf(positional("PATH"), many)]
        paths: Vec<OsString>,
    },
    /// Run various checks on a set of files.
    #[bpaf(command)]
    Lint {
        /// Writes safe fixes
        #[bpaf(long("write"), switch)]
        write: bool,

        /// Allow to do unsafe fixes, should be used with `--write` or `--fix`
        #[bpaf(long("unsafe"), switch)]
        unsafe_: bool,

        /// Alias for `--write`, writes safe fixes
        #[bpaf(long("fix"), switch, hide_usage)]
        fix: bool,

        /// Alias for `--write`, writes safe fixes
        #[bpaf(long("apply"), switch, hide_usage)]
        apply: bool,

        /// Alias for `--write --unsafe`, writes safe and unsafe fixes
        #[bpaf(long("apply-unsafe"), switch, hide_usage)]
        apply_unsafe: bool,

        #[bpaf(external(partial_linter_configuration), hide_usage, optional)]
        linter_configuration: Option<PartialLinterConfiguration>,

        #[bpaf(external(partial_vcs_configuration), optional, hide_usage)]
        vcs_configuration: Option<PartialVcsConfiguration>,

        #[bpaf(external(partial_files_configuration), optional, hide_usage)]
        files_configuration: Option<PartialFilesConfiguration>,

        #[bpaf(external(partial_javascript_linter), optional, hide_usage)]
        javascript_linter: Option<PartialJavascriptLinter>,

        #[bpaf(external(partial_json_linter), optional, hide_usage)]
        json_linter: Option<PartialJsonLinter>,

        #[bpaf(external(partial_css_linter), optional, hide_usage, hide)]
        css_linter: Option<PartialCssLinter>,

        #[bpaf(external, hide_usage)]
        cli_options: CliOptions,

        /// Run only the given rule or rule group.
        ///
        /// The option overrides the Biome configuration file as follows:
        ///
        /// - When a rule is passed, its severity level is set to `error' if it is a recommended rule, or `warn' otherwise.
        ///
        /// - When a rule group is passed, the `recommended` flag is enabled, but if the `all` flag is enabled.
        ///
        /// Example: `biome lint --rule=correctness/noUnusedVariables`
        ///
        /// Example: `biome lint --rule=suspicious`
        #[bpaf(long("rule"), argument("GROUP|RULE"))]
        rule: Option<RuleSelector>,

        /// Use this option when you want to format code piped from `stdin`, and print the output to `stdout`.
        ///
        /// The file doesn't need to exist on disk, what matters is the extension of the file. Based on the extension, Biome knows how to lint the code.
        ///
        /// Example: `echo 'let a;' | biome lint --stdin-file-path=file.js`
        #[bpaf(long("stdin-file-path"), argument("PATH"), hide_usage)]
        stdin_file_path: Option<String>,
        /// When set to true, only the files that have been staged (the ones prepared to be committed)
        /// will be linted.
        #[bpaf(long("staged"), switch)]
        staged: bool,
        /// When set to true, only the files that have been changed compared to your `defaultBranch`
        /// configuration will be linted.
        #[bpaf(long("changed"), switch)]
        changed: bool,
        /// Use this to specify the base branch to compare against when you're using the --changed
        /// flag and the `defaultBranch` is not set in your biome.json
        #[bpaf(long("since"), argument("REF"))]
        since: Option<String>,
        /// Single file, single path or list of paths
        #[bpaf(positional("PATH"), many)]
        paths: Vec<OsString>,
    },
    /// Run the formatter on a set of files.
    #[bpaf(command)]
    Format {
        #[bpaf(external(partial_formatter_configuration), optional, hide_usage)]
        formatter_configuration: Option<PartialFormatterConfiguration>,

        #[bpaf(external(partial_javascript_formatter), optional, hide_usage)]
        javascript_formatter: Option<PartialJavascriptFormatter>,

        #[bpaf(external(partial_json_formatter), optional, hide_usage)]
        json_formatter: Option<PartialJsonFormatter>,

        #[bpaf(external(partial_css_formatter), optional, hide_usage, hide)]
        css_formatter: Option<PartialCssFormatter>,

        #[bpaf(external(partial_vcs_configuration), optional, hide_usage)]
        vcs_configuration: Option<PartialVcsConfiguration>,

        #[bpaf(external(partial_files_configuration), optional, hide_usage)]
        files_configuration: Option<PartialFilesConfiguration>,
        /// Use this option when you want to format code piped from `stdin`, and print the output to `stdout`.
        ///
        /// The file doesn't need to exist on disk, what matters is the extension of the file. Based on the extension, Biome knows how to format the code.
        ///
        /// Example: `echo 'let a;' | biome format --stdin-file-path=file.js`
        #[bpaf(long("stdin-file-path"), argument("PATH"), hide_usage)]
        stdin_file_path: Option<String>,

        #[bpaf(external, hide_usage)]
        cli_options: CliOptions,

        /// Writes formatted files to file system.
        #[bpaf(long("write"), switch)]
        write: bool,

        /// Alias of `--write`, writes formatted files to file system.
        #[bpaf(long("fix"), switch, hide_usage)]
        fix: bool,

        /// When set to true, only the files that have been staged (the ones prepared to be committed)
        /// will be linted.
        #[bpaf(long("staged"), switch)]
        staged: bool,

        /// When set to true, only the files that have been changed compared to your `defaultBranch`
        /// configuration will be linted.
        #[bpaf(long("changed"), switch)]
        changed: bool,

        /// Use this to specify the base branch to compare against when you're using the --changed
        /// flag and the `defaultBranch` is not set in your biome.json
        #[bpaf(long("since"), argument("REF"))]
        since: Option<String>,

        /// Single file, single path or list of paths.
        #[bpaf(positional("PATH"), many)]
        paths: Vec<OsString>,
    },
    /// Command to use in CI environments. Runs formatter, linter and import sorting to the requested files.
    ///
    /// Files won't be modified, the command is a read-only operation.
    #[bpaf(command)]
    Ci {
        /// Allow to enable or disable the formatter check.
        #[bpaf(long("formatter-enabled"), argument("true|false"), optional)]
        formatter_enabled: Option<bool>,
        /// Allow to enable or disable the linter check.
        #[bpaf(long("linter-enabled"), argument("true|false"), optional)]
        linter_enabled: Option<bool>,
        /// Allow to enable or disable the organize imports.
        #[bpaf(long("organize-imports-enabled"), argument("true|false"), optional)]
        organize_imports_enabled: Option<bool>,

        #[bpaf(external(partial_configuration), hide_usage, optional)]
        configuration: Option<PartialConfiguration>,
        #[bpaf(external, hide_usage)]
        cli_options: CliOptions,

        /// When set to true, only the files that have been changed compared to your `defaultBranch`
        /// configuration will be linted.
        #[bpaf(long("changed"), switch)]
        changed: bool,

        /// Use this to specify the base branch to compare against when you're using the --changed
        /// flag and the `defaultBranch` is not set in your biome.json
        #[bpaf(long("since"), argument("REF"))]
        since: Option<String>,

        /// Single file, single path or list of paths
        #[bpaf(positional("PATH"), many)]
        paths: Vec<OsString>,
    },

    /// Bootstraps a new biome project. Creates a configuration file with some defaults.
    #[bpaf(command)]
    Init(
        /// Tells Biome to emit a `biome.jsonc` file.
        #[bpaf(long("jsonc"), switch)]
        bool,
    ),
    /// Acts as a server for the Language Server Protocol over stdin/stdout
    #[bpaf(command("lsp-proxy"))]
    LspProxy(
        /// Allows to set a custom file path to the configuration file,
        /// or a custom directory path to find `biome.json` or `biome.jsonc`
        #[bpaf(env("BIOME_CONFIG_PATH"), long("config-path"), argument("PATH"))]
        Option<PathBuf>,
        /// Bogus argument to make the command work with vscode-languageclient
        #[bpaf(long("stdio"), hide, hide_usage, switch)]
        bool,
    ),
    /// It updates the configuration when there are breaking changes
    #[bpaf(command)]
    Migrate {
        #[bpaf(external, hide_usage)]
        cli_options: CliOptions,

        /// Writes the new configuration file to disk
        #[bpaf(long("write"), switch)]
        write: bool,

        /// Alias of `--write`, writes the new configuration file to disk
        #[bpaf(long("fix"), switch, hide_usage)]
        fix: bool,

        #[bpaf(external(migrate_sub_command), optional)]
        sub_command: Option<MigrateSubCommand>,
    },

    /// Searches for Grit patterns across a project.
    #[bpaf(command, hide)] // !! Command is hidden until ready for release.
    Search {
        #[bpaf(external, hide_usage)]
        cli_options: CliOptions,

        #[bpaf(external(partial_files_configuration), optional, hide_usage)]
        files_configuration: Option<PartialFilesConfiguration>,

        #[bpaf(external(partial_vcs_configuration), optional, hide_usage)]
        vcs_configuration: Option<PartialVcsConfiguration>,

        /// Use this option when you want to search through code piped from
        /// `stdin`, and print the output to `stdout`.
        ///
        /// The file doesn't need to exist on disk, what matters is the
        /// extension of the file. Based on the extension, Biome knows how to
        /// parse the code.
        ///
        /// Example: `echo 'let a;' | biome search '`let $var`' --stdin-file-path=file.js`
        #[bpaf(long("stdin-file-path"), argument("PATH"), hide_usage)]
        stdin_file_path: Option<String>,

        /// The GritQL pattern to search for.
        ///
        /// Note that the search command (currently) does not support rewrites.
        #[bpaf(positional("PATH"))]
        pattern: String,

        /// Single file, single path or list of paths.
        #[bpaf(positional("PATH"), many)]
        paths: Vec<OsString>,
    },

    /// A command to retrieve the documentation of various aspects of the CLI.
    ///
    /// ## Examples
    ///
    /// ```shell
    /// biome explain noDebugger
    /// ```
    ///
    /// ```shell
    /// biome explain daemon-logs
    /// ```
    #[bpaf(command)]
    Explain {
        /// Single name to display documentation for.
        #[bpaf(positional("NAME"))]
        doc: Doc,
    },

    #[bpaf(command)]
    /// Clean the logs emitted by the daemon
    Clean,

    #[bpaf(command("__run_server"), hide)]
    RunServer {
        #[bpaf(long("stop-on-disconnect"), hide_usage)]
        stop_on_disconnect: bool,
        /// Allows to set a custom file path to the configuration file,
        /// or a custom directory path to find `biome.json` or `biome.jsonc`
        #[bpaf(env("BIOME_CONFIG_PATH"), long("config-path"), argument("PATH"))]
        config_path: Option<PathBuf>,
    },
    #[bpaf(command("__print_socket"), hide)]
    PrintSocket,
}

#[derive(Debug, Bpaf, Clone)]
pub enum MigrateSubCommand {
    /// It attempts to find the files `.prettierrc`/`prettier.json` and `.prettierignore`, and map the Prettier's configuration into Biome's configuration file.
    #[bpaf(command)]
    Prettier,
    /// It attempts to find the ESLint configuration file in the working directory, and update the Biome's configuration file as a result.
    #[bpaf(command)]
    Eslint {
        /// Includes rules inspired from an eslint rule in the migration
        #[bpaf(long("include-inspired"))]
        include_inspired: bool,
        /// Includes nursery rules in the migration
        #[bpaf(long("include-nursery"))]
        include_nursery: bool,
    },
}

impl MigrateSubCommand {
    pub const fn is_prettier(&self) -> bool {
        matches!(self, MigrateSubCommand::Prettier)
    }
}

impl BiomeCommand {
    const fn cli_options(&self) -> Option<&CliOptions> {
        match self {
            BiomeCommand::Version(cli_options)
            | BiomeCommand::Rage(cli_options, ..)
            | BiomeCommand::Check { cli_options, .. }
            | BiomeCommand::Lint { cli_options, .. }
            | BiomeCommand::Ci { cli_options, .. }
            | BiomeCommand::Format { cli_options, .. }
            | BiomeCommand::Migrate { cli_options, .. }
            | BiomeCommand::Search { cli_options, .. } => Some(cli_options),
            BiomeCommand::LspProxy(_, _)
            | BiomeCommand::Start(_)
            | BiomeCommand::Stop
            | BiomeCommand::Init(_)
            | BiomeCommand::Explain { .. }
            | BiomeCommand::RunServer { .. }
            | BiomeCommand::Clean { .. }
            | BiomeCommand::PrintSocket => None,
        }
    }

    pub const fn get_color(&self) -> Option<&ColorsArg> {
        match self.cli_options() {
            Some(cli_options) => cli_options.colors.as_ref(),
            None => None,
        }
    }

    pub const fn should_use_server(&self) -> bool {
        match self.cli_options() {
            Some(cli_options) => cli_options.use_server,
            None => false,
        }
    }

    pub const fn has_metrics(&self) -> bool {
        false
    }

    pub fn is_verbose(&self) -> bool {
        self.cli_options()
            .map_or(false, |cli_options| cli_options.verbose)
    }

    pub fn log_level(&self) -> LoggingLevel {
        self.cli_options()
            .map_or(LoggingLevel::default(), |cli_options| cli_options.log_level)
    }

    pub fn log_kind(&self) -> LoggingKind {
        self.cli_options()
            .map_or(LoggingKind::default(), |cli_options| cli_options.log_kind)
    }
}

/// It accepts a [LoadedPartialConfiguration] and it prints the diagnostics emitted during parsing and deserialization.
///
/// If it contains errors, it return an error.
pub(crate) fn validate_configuration_diagnostics(
    loaded_configuration: &LoadedConfiguration,
    console: &mut dyn Console,
    verbose: bool,
) -> Result<(), CliDiagnostic> {
    if let Some(file_path) = loaded_configuration
        .file_path
        .as_ref()
        .and_then(|f| f.file_name())
        .and_then(|f| f.to_str())
    {
        if file_path == "rome.json" {
            let diagnostic = DeprecatedConfigurationFile::new(file_path);
            if diagnostic.tags().is_verbose() && verbose {
                console.error(markup! {{PrintDiagnostic::verbose(&diagnostic)}})
            } else {
                console.error(markup! {{PrintDiagnostic::simple(&diagnostic)}})
            }
        }
    }
    let diagnostics = loaded_configuration.as_diagnostics_iter();
    for diagnostic in diagnostics {
        if diagnostic.tags().is_verbose() && verbose {
            console.error(markup! {{PrintDiagnostic::verbose(diagnostic)}})
        } else {
            console.error(markup! {{PrintDiagnostic::simple(diagnostic)}})
        }
    }

    if loaded_configuration.has_errors() {
        return Err(CliDiagnostic::workspace_error(
            BiomeDiagnostic::invalid_configuration(
                "Biome exited because the configuration resulted in errors. Please fix them.",
            )
            .into(),
        ));
    }

    Ok(())
}

fn resolve_manifest(cli_session: &CliSession) -> Result<(), WorkspaceError> {
    let fs = &*cli_session.app.fs;
    let workspace = &*cli_session.app.workspace;

    let result = fs.auto_search(
        &fs.working_directory().unwrap_or_default(),
        &["package.json"],
        false,
    )?;

    if let Some(result) = result {
        let biome_path = BiomePath::new(result.file_path);
        workspace.open_project(OpenProjectParams {
            path: biome_path.clone(),
            content: result.content,
            version: 0,
        })?;
        workspace.update_current_project(UpdateProjectParams { path: biome_path })?;
    }

    Ok(())
}

/// Computes [Stdin] if the CLI has the necessary information.
///
/// ## Errors
/// - If the user didn't provide anything via `stdin` but the option `--stdin-file-path` is passed.
pub(crate) fn get_stdin(
    stdin_file_path: Option<String>,
    console: &mut dyn Console,
    command_name: &str,
) -> Result<Option<Stdin>, CliDiagnostic> {
    let stdin = if let Some(stdin_file_path) = stdin_file_path {
        let input_code = console.read();
        if let Some(input_code) = input_code {
            let path = PathBuf::from(stdin_file_path);
            Some((path, input_code).into())
        } else {
            // we provided the argument without a piped stdin, we bail
            return Err(CliDiagnostic::missing_argument("stdin", command_name));
        }
    } else {
        None
    };

    Ok(stdin)
}

fn get_files_to_process(
    since: Option<String>,
    changed: bool,
    staged: bool,
    fs: &DynRef<'_, dyn FileSystem>,
    configuration: &PartialConfiguration,
) -> Result<Option<Vec<OsString>>, CliDiagnostic> {
    if since.is_some() {
        if !changed {
            return Err(CliDiagnostic::incompatible_arguments("since", "changed"));
        }
        if staged {
            return Err(CliDiagnostic::incompatible_arguments("since", "staged"));
        }
    }

    if changed {
        if staged {
            return Err(CliDiagnostic::incompatible_arguments("changed", "staged"));
        }
        Ok(Some(get_changed_files(fs, configuration, since)?))
    } else if staged {
        Ok(Some(get_staged_files(fs)?))
    } else {
        Ok(None)
    }
}

/// Holds the options to determine the fix file mode.
pub(crate) struct FixFileModeOptions {
    apply: bool,
    apply_unsafe: bool,
    write: bool,
    fix: bool,
    unsafe_: bool,
}

/// - [Result]: if the given options are incompatible
/// - [Option]: if no fixes are requested
/// - [FixFileMode]: if safe or unsafe fixes are requested
pub(crate) fn determine_fix_file_mode(
    options: FixFileModeOptions,
    _console: &mut dyn Console,
) -> Result<Option<FixFileMode>, CliDiagnostic> {
    let FixFileModeOptions {
        apply,
        apply_unsafe,
        write,
        fix,
        unsafe_,
    } = options;

    check_fix_incompatible_arguments(options)?;

    let safe_fixes = apply || write || fix;
    let unsafe_fixes = apply_unsafe || ((write || safe_fixes) && unsafe_);

    if unsafe_fixes {
        Ok(Some(FixFileMode::SafeAndUnsafeFixes))
    } else if safe_fixes {
        Ok(Some(FixFileMode::SafeFixes))
    } else {
        Ok(None)
    }
}

/// Checks if the fix file options are incompatible.
fn check_fix_incompatible_arguments(options: FixFileModeOptions) -> Result<(), CliDiagnostic> {
    let FixFileModeOptions {
        apply,
        apply_unsafe,
        write,
        fix,
        unsafe_,
    } = options;
    if apply && apply_unsafe {
        return Err(CliDiagnostic::incompatible_arguments(
            "--apply",
            "--apply-unsafe",
        ));
    } else if apply_unsafe && unsafe_ {
        return Err(CliDiagnostic::incompatible_arguments(
            "--apply-unsafe",
            "--unsafe",
        ));
    } else if apply && (fix || write) {
        return Err(CliDiagnostic::incompatible_arguments(
            "--apply",
            if fix { "--fix" } else { "--write" },
        ));
    } else if apply_unsafe && (fix || write) {
        return Err(CliDiagnostic::incompatible_arguments(
            "--apply-unsafe",
            if fix { "--fix" } else { "--write" },
        ));
    } else if write && fix {
        return Err(CliDiagnostic::incompatible_arguments("--write", "--fix"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use biome_console::BufferConsole;

    use super::*;

    #[test]
    fn incompatible_arguments() {
        for (apply, apply_unsafe, write, fix, unsafe_) in [
            (true, true, false, false, false), // --apply --apply-unsafe
            (true, false, true, false, false), // --apply --write
            (true, false, false, true, false), // --apply --fix
            (false, true, false, false, true), // --apply-unsafe --unsafe
            (false, true, true, false, false), // --apply-unsafe --write
            (false, true, false, true, false), // --apply-unsafe --fix
            (false, false, true, true, false), // --write --fix
        ] {
            assert!(check_fix_incompatible_arguments(FixFileModeOptions {
                apply,
                apply_unsafe,
                write,
                fix,
                unsafe_
            })
            .is_err());
        }
    }

    #[test]
    fn safe_fixes() {
        let mut console = BufferConsole::default();

        for (apply, apply_unsafe, write, fix, unsafe_) in [
            (true, false, false, false, false), // --apply
            (false, false, true, false, false), // --write
            (false, false, false, true, false), // --fix
        ] {
            assert_eq!(
                determine_fix_file_mode(
                    FixFileModeOptions {
                        apply,
                        apply_unsafe,
                        write,
                        fix,
                        unsafe_
                    },
                    &mut console
                )
                .unwrap(),
                Some(FixFileMode::SafeFixes)
            );
        }
    }

    #[test]
    fn safe_and_unsafe_fixes() {
        let mut console = BufferConsole::default();

        for (apply, apply_unsafe, write, fix, unsafe_) in [
            (false, true, false, false, false), // --apply-unsafe
            (false, false, true, false, true),  // --write --unsafe
            (false, false, false, true, true),  // --fix --unsafe
        ] {
            assert_eq!(
                determine_fix_file_mode(
                    FixFileModeOptions {
                        apply,
                        apply_unsafe,
                        write,
                        fix,
                        unsafe_
                    },
                    &mut console
                )
                .unwrap(),
                Some(FixFileMode::SafeAndUnsafeFixes)
            );
        }
    }

    #[test]
    fn no_fix() {
        let mut console = BufferConsole::default();

        let (apply, apply_unsafe, write, fix, unsafe_) = (false, false, false, false, false);
        assert_eq!(
            determine_fix_file_mode(
                FixFileModeOptions {
                    apply,
                    apply_unsafe,
                    write,
                    fix,
                    unsafe_
                },
                &mut console
            )
            .unwrap(),
            None
        );
    }

    /// Tests that all CLI options adhere to the invariants expected by `bpaf`.
    #[test]
    fn check_options() {
        biome_command().check_invariants(false);
    }
}
