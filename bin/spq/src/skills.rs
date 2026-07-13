use anyhow::Result;
use clap::{Subcommand, ValueEnum};
use std::path::{Path, PathBuf};

const SKILL_MD: &str = include_str!("../../../.claude/skills/spq-analytics/SKILL.md");
const SKILL_NAME: &str = "spq-analytics";

/// Coding agents that load skills from a well-known `SKILL.md` directory layout.
#[derive(Clone, Copy, Debug, ValueEnum, PartialEq, Eq)]
pub enum Provider {
    /// Claude Code (`~/.claude/skills`)
    Claude,
    /// Grok (`~/.grok/skills`)
    Grok,
    /// Cursor (`~/.cursor/skills`)
    Cursor,
    /// Generic Agent Skills path (`~/.agents/skills`)
    Agents,
}

impl Provider {
    fn all() -> &'static [Provider] {
        &[
            Provider::Claude,
            Provider::Grok,
            Provider::Cursor,
            Provider::Agents,
        ]
    }

    fn label(self) -> &'static str {
        match self {
            Provider::Claude => "Claude Code",
            Provider::Grok => "Grok",
            Provider::Cursor => "Cursor",
            Provider::Agents => "agents",
        }
    }

    /// Relative path under `$HOME` for this provider's global skills root.
    fn skills_root_rel(self) -> &'static str {
        match self {
            Provider::Claude => ".claude/skills",
            Provider::Grok => ".grok/skills",
            Provider::Cursor => ".cursor/skills",
            Provider::Agents => ".agents/skills",
        }
    }

    fn skill_dir(self, home: &Path) -> PathBuf {
        home.join(self.skills_root_rel()).join(SKILL_NAME)
    }
}

#[derive(Subcommand)]
pub enum SkillsCommand {
    /// Install the spq-analytics skill for coding agents that support SKILL.md.
    Install {
        /// Overwrite an existing installation without prompting.
        #[arg(long)]
        force: bool,
        /// Install only for a specific agent (default: all known providers).
        #[arg(long, value_enum)]
        provider: Option<Provider>,
    },
}

pub fn run(cmd: &SkillsCommand) -> Result<()> {
    match cmd {
        SkillsCommand::Install { force, provider } => install(*force, *provider),
    }
}

fn home_dir() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into()))
}

fn install(force: bool, provider: Option<Provider>) -> Result<()> {
    let home = home_dir();
    let targets: Vec<Provider> = match provider {
        Some(p) => vec![p],
        None => Provider::all().to_vec(),
    };

    // When installing to every provider, skip targets that already exist unless
    // --force is set, but still install the rest. When a single provider is
    // named and it already exists, fail like before so scripts get a clear signal.
    let single = targets.len() == 1;
    let mut installed = 0usize;
    let mut skipped = 0usize;

    for p in targets {
        let dir = p.skill_dir(&home);
        let dest = dir.join("SKILL.md");

        if dest.exists() && !force {
            if single {
                anyhow::bail!(
                    "{} already exists. Re-run with --force to overwrite.",
                    dest.display()
                );
            }
            println!("Skipped (exists): {} [{}]", dest.display(), p.label());
            skipped += 1;
            continue;
        }

        std::fs::create_dir_all(&dir)?;
        std::fs::write(&dest, SKILL_MD)?;
        println!("Installed: {} [{}]", dest.display(), p.label());
        installed += 1;
    }

    if installed == 0 && skipped > 0 {
        anyhow::bail!(
            "Nothing installed; all targets already exist. Re-run with --force to overwrite."
        );
    }

    println!(
        "Done. Restart your coding agent session so it reloads skills ({installed} installed, {skipped} skipped)."
    );
    Ok(())
}
