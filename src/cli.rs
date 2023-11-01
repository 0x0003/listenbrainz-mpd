use std::path::PathBuf;

use clap::{ArgAction, Parser};

#[derive(Parser)]
#[clap(version, about)]
pub struct CliArgs {
    /// Path to the configuration file.
    #[clap(short, long)]
    pub config: Option<PathBuf>,
    /// Create a configuration file in the default location and exit
    #[clap(long, action = ArgAction::SetTrue, exclusive = true)]
    pub create_default_config: bool,
    /// Submit feedback for the currently playing song and exit.
    #[clap(long, exclusive = true, value_enum, value_name = "FEEDBACK")]
    pub send_feedback: Option<Feedback>,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum Feedback {
    Hate,
    Clear,
    Love,
}

impl Feedback {
    pub fn from_command(s: &str) -> Option<Feedback> {
        match s {
            "love" => Some(Feedback::Love),
            "hate" => Some(Feedback::Hate),
            "clear" => Some(Feedback::Clear),
            _ => None,
        }
    }

    pub fn as_command(&self) -> &'static str {
        match self {
            Feedback::Hate => "hate",
            Feedback::Clear => "clear",
            Feedback::Love => "love",
        }
    }
}
