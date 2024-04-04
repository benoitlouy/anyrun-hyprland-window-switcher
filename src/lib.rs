use abi_stable::std_types::{ROption, RString, RVec};
use anyrun_plugin::*;
use fuzzy_matcher::FuzzyMatcher;
use nix::unistd;
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::io;
use std::process::Command;

mod scrubber;

#[derive(Deserialize)]
struct Config {
    #[serde(default = "max_entries")]
    max_entries: usize,
    #[serde(default = "hyprctl_path")]
    hyprctl_path: String,
    #[serde(default = "prefix")]
    prefix: String,
}

fn max_entries() -> usize {
    10
}

fn hyprctl_path() -> String {
    "hyprctl".into()
}

fn prefix() -> String {
    "".into()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            max_entries: max_entries(),
            hyprctl_path: hyprctl_path(),
            prefix: prefix(),
        }
    }
}

struct State {
    config: Config,
    clients: Vec<(u64, HyprClient)>,
    desktop_entries: HashMap<String, scrubber::DesktopEntry>,
}

#[derive(Deserialize, Debug)]
struct HyprClient {
    address: String,
    #[serde(rename = "initialTitle")]
    initial_title: String,
    title: String,
    #[serde(rename = "initialClass")]
    initial_class: String,
    class: String,
    mapped: bool,
}

#[derive(Debug)]
enum Error {
    HyprctlCommandFailed(io::Error),
    HyprctlReturnCodeError(i32),
    ReadOutputError(std::string::FromUtf8Error),
    ParsingError(serde_json::Error),
}

const CLIENT_ARGS: [&str; 2] = ["clients", "-j"];

#[init]
fn init(config_dir: RString) -> State {
    let config: Config = load_config(config_dir);

    let content = execute_command(&config.hyprctl_path, &CLIENT_ARGS);

    let desktop_entries = scrubber::scrubber()
        .unwrap_or_else(|why| {
            eprintln!("Failed to load desktop entries: {}", why);
            Vec::new()
        })
        .into_iter()
        .map(|e| (e.name.to_lowercase(), e))
        .collect::<HashMap<_, _>>();

    let hyprctl_clients = content
        .and_then(|s| {
            serde_json::from_str::<Vec<HyprClient>>(s.as_str()).map_err(Error::ParsingError)
        })
        .map(|clients| {
            clients
                .into_iter()
                .enumerate()
                .filter_map(|(id, client)| {
                    if client.mapped {
                        Some((id as u64, client))
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
        });

    hyprctl_clients
        .map(|clients| State {
            config,
            clients,
            desktop_entries,
        })
        .unwrap()
}

fn load_config(config_dir: RString) -> Config {
    match fs::read_to_string(format!("{}/hyprland_window_switcher.ron", config_dir)) {
        Ok(content) => ron::from_str(&content).unwrap_or_else(|why| {
            eprintln!(
                "Error parsing hyprland window switcher plugin config: {}",
                why
            );
            Config::default()
        }),
        Err(why) => {
            eprintln!(
                "Error reading hyprland window switcher plugin config: {}",
                why
            );
            Config::default()
        }
    }
}

fn execute_command(cmd: &str, args: &[&str]) -> Result<String, Error> {
    let output = Command::new(cmd)
        .args(args)
        .output()
        .map_err(Error::HyprctlCommandFailed);

    output.and_then(|o| {
        if o.status.success() {
            String::from_utf8(o.stdout).map_err(Error::ReadOutputError)
        } else {
            Err(Error::HyprctlReturnCodeError(o.status.code().unwrap()))
        }
    })
}

#[info]
fn info() -> PluginInfo {
    PluginInfo {
        name: "Hyprland window switcher".into(),
        icon: "help-about".into(), // Icon from the icon theme
    }
}

#[get_matches]
fn get_matches(input: RString, state: &State) -> RVec<Match> {
    if !input.starts_with(&state.config.prefix) {
        return RVec::new();
    }

    let cleaned_input = &input[state.config.prefix.len()..];
    if cleaned_input.is_empty() {
        state
            .clients
            .iter()
            .map(|(id, e)| build_match(e, *id, &state.desktop_entries))
            .collect()
    } else {
        let matcher = fuzzy_matcher::skim::SkimMatcherV2::default().smart_case();

        let mut entries = state
            .clients
            .iter()
            .filter_map(|(id, e)| {
                let score = matcher
                    .fuzzy_match(&e.initial_title, cleaned_input)
                    .unwrap_or(0);
                if score > 0 {
                    Some((id, e, score))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();

        entries.sort_by(|a, b| b.2.cmp(&a.2));
        entries.truncate(state.config.max_entries);

        entries
            .into_iter()
            .map(|(id, e, _)| build_match(e, *id, &state.desktop_entries))
            .collect()
    }
}

fn build_match(
    client: &HyprClient,
    id: u64,
    desktop_entries: &HashMap<String, scrubber::DesktopEntry>,
) -> Match {
    let icon: ROption<RString> = desktop_entries
        .get(&client.class.to_lowercase())
        .or_else(|| desktop_entries.get(&client.title.to_lowercase()))
        .or_else(|| desktop_entries.get(&client.initial_class.to_lowercase()))
        .or_else(|| desktop_entries.get(&client.initial_title.to_lowercase()))
        .map(|e| e.icon.clone().into())
        .into();

    Match {
        title: client.initial_title.clone().into(),
        icon,
        use_pango: false,
        description: description(client),
        id: ROption::RSome(id),
    }
}

fn description(client: &HyprClient) -> ROption<RString> {
    if client.title != client.initial_title {
        let desc = if client.title.len() > 75 {
            let mut desc = client.title.clone();
            desc.truncate(75);
            format!("{}...", desc)
        } else {
            client.title.clone()
        };
        ROption::RSome(desc.into())
    } else {
        ROption::RNone
    }
}

#[handler]
fn handler(selection: Match, state: &State) -> HandleResult {
    let client = state
        .clients
        .iter()
        .find_map(|(id, client)| {
            if *id == selection.id.unwrap() {
                Some(client)
            } else {
                None
            }
        })
        .unwrap();

    match unsafe { unistd::fork() } {
        Ok(unistd::ForkResult::Child) => {
            std::thread::sleep(std::time::Duration::from_millis(150));
            execute_command(
                &state.config.hyprctl_path,
                &[
                    "dispatch",
                    "focuswindow",
                    format!("address:{}", client.address).as_str(),
                ],
            ).unwrap();
            unsafe { libc::exit(0) };
        }
        Ok(..) => {
            HandleResult::Close
        }
        Err(why) => {
            eprintln!("Failed to fork {}", why);
            HandleResult::Close
        }
    }
}
