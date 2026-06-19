use anyhow::Result;
use clap::Subcommand;
use std::path::PathBuf;

const SKILL_MD: &str = include_str!("../../../.claude/skills/spq-analytics/SKILL.md");
const SKILL_NAME: &str = "spq-analytics";

#[derive(Subcommand)]
pub enum SkillsCommand {
    /// Install the spq-analytics skill into ~/.claude/skills so Claude Code picks it up globally.
    Install {
        /// Overwrite an existing installation without prompting.
        #[arg(long)]
        force: bool,
    },
}

pub fn run(cmd: &SkillsCommand) -> Result<()> {
    match cmd {
        SkillsCommand::Install { force } => install(*force),
    }
}

fn skill_dir() -> Result<PathBuf> {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    Ok(PathBuf::from(home)
        .join(".claude")
        .join("skills")
        .join(SKILL_NAME))
}

fn install(force: bool) -> Result<()> {
    let dir = skill_dir()?;
    let dest = dir.join("SKILL.md");

    if dest.exists() && !force {
        anyhow::bail!(
            "{} already exists. Re-run with --force to overwrite.",
            dest.display()
        );
    }

    std::fs::create_dir_all(&dir)?;
    std::fs::write(&dest, SKILL_MD)?;
    println!("Installed: {}", dest.display());
    println!("Claude Code will pick up the skill on next session start.");
    Ok(())
}
