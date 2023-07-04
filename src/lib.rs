use abi_stable::std_types::{ROption, RString, RVec};
use anyrun_plugin::*;
use fuzzy_matcher::FuzzyMatcher;
use serde::{Deserialize, Deserializer};
use std::fs;
use std::io;
use std::process::Command;
use url::Url;

#[derive(Deserialize)]
struct Config {
    #[serde(default = "max_entries")]
    max_entries: usize,
    #[serde(default = "prefix")]
    prefix: String,
}

fn max_entries() -> usize {
    10
}

fn prefix() -> String {
    "".into()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            max_entries: max_entries(),
            prefix: prefix(),
        }
    }
}

#[derive(Debug)]
enum Error {
    HyprctlCommandFailed(io::Error),
}

struct State {
    config: Config,
}

fn execute_command(cmd: &str, args: &[&str]) -> Result<String, Error> {
    let output = Command::new(cmd)
        .args(args)
        .output()
        .map_err(Error::OpCommandFailed);

    output.and_then(|o| {
        if o.status.success() {
            String::from_utf8(o.stdout).map_err(Error::ReadOutputError)
        } else {
            Err(Error::OpReturnCodeError(o.status.code().unwrap()))
        }
    })
}

#[init]
fn init(config_dir: RString) -> State {
    let config: Config = load_config(config_dir);

}

#[info]
fn info() -> PluginInfo {
    PluginInfo {
        name: "hyprland window switcher".into(),
        icon: "1password".into(), // Icon from the icon theme
    }
}

fn load_config(config_dir: RString) -> Config {
    match fs::read_to_string(format!("{}/hyprland_window_switcher.ron", config_dir)) {
        Ok(content) => ron::from_str(&content).unwrap_or_else(|why| {
            eprintln!("Error parsing op plugin config: {}", why);
            Config::default()
        }),
        Err(why) => {
            eprintln!("Error reading op plugin config: {}", why);
            Config::default()
        }
    }
}

#[get_matches]
fn get_matches(input: RString, state: &mut State) -> RVec<Match> {
    match &state.selection {
        None => display_matching_items(&input, state),
        Some(selection) => match &state.input {
            None => display_matching_items(&input, state),
            Some(s) => {
                if input.as_str() == s {
                    display_selection_items(selection)
                } else {
                    state.selection = None;
                    state.input = None;
                    display_matching_items(&input, state)
                }
            }
        },
    }
}

#[handler]
fn handler(selection: Match, state: &mut State) -> HandleResult {
}
