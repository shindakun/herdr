use std::collections::HashMap;

use crate::api::schema::{
    TabCreateParams, TabListParams, TabMoveDestination, TabMoveToWorkspaceParams, TabRenameParams,
};

pub(super) fn run_tab_command(args: &[String]) -> std::io::Result<i32> {
    let Some(subcommand) = args.first().map(|arg| arg.as_str()) else {
        print_tab_help();
        return Ok(2);
    };

    match subcommand {
        "list" => tab_list(&args[1..]),
        "create" => tab_create(&args[1..]),
        "get" => tab_get(&args[1..]),
        "focus" => tab_focus(&args[1..]),
        "rename" => tab_rename(&args[1..]),
        "move-to-workspace" => tab_move_to_workspace(&args[1..]),
        "close" => tab_close(&args[1..]),
        "help" | "--help" | "-h" => {
            print_tab_help();
            Ok(0)
        }
        _ => {
            print_tab_help();
            Ok(2)
        }
    }
}

fn tab_list(args: &[String]) -> std::io::Result<i32> {
    let mut workspace_id = None;

    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--workspace" => {
                let Some(value) = args.get(index + 1) else {
                    eprintln!("missing value for --workspace");
                    return Ok(2);
                };
                workspace_id = Some(super::normalize_workspace_id(value));
                index += 2;
            }
            other => {
                eprintln!("unknown option: {other}");
                return Ok(2);
            }
        }
    }

    super::runtime::tab_list(TabListParams { workspace_id })
}

fn tab_create(args: &[String]) -> std::io::Result<i32> {
    let mut workspace_id = None;
    let mut cwd = None;
    let mut focus = false;
    let mut label = None;
    let mut env = HashMap::new();

    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--workspace" => {
                let Some(value) = args.get(index + 1) else {
                    eprintln!("missing value for --workspace");
                    return Ok(2);
                };
                workspace_id = Some(super::normalize_workspace_id(value));
                index += 2;
            }
            "--cwd" => {
                let Some(value) = args.get(index + 1) else {
                    eprintln!("missing value for --cwd");
                    return Ok(2);
                };
                cwd = Some(value.clone());
                index += 2;
            }
            "--label" => {
                let Some(value) = args.get(index + 1) else {
                    eprintln!("missing value for --label");
                    return Ok(2);
                };
                label = Some(value.clone());
                index += 2;
            }
            "--focus" => {
                focus = true;
                index += 1;
            }
            "--no-focus" => {
                focus = false;
                index += 1;
            }
            "--env" => {
                let Some(value) = args.get(index + 1) else {
                    eprintln!("missing value for --env");
                    return Ok(2);
                };
                let (key, value) = match super::parse_env_assignment(value) {
                    Ok(pair) => pair,
                    Err(err) => {
                        eprintln!("{err}");
                        return Ok(2);
                    }
                };
                env.insert(key, value);
                index += 2;
            }
            other => {
                eprintln!("unknown option: {other}");
                return Ok(2);
            }
        }
    }

    super::runtime::tab_create(TabCreateParams {
        workspace_id,
        cwd,
        focus,
        label,
        env,
    })
}

fn tab_get(args: &[String]) -> std::io::Result<i32> {
    let Some(raw_tab_id) = args.first() else {
        eprintln!("usage: herdr tab get <tab_id>");
        return Ok(2);
    };
    if args.len() != 1 {
        eprintln!("usage: herdr tab get <tab_id>");
        return Ok(2);
    }

    super::runtime::tab_get(super::normalize_tab_id(raw_tab_id))
}

fn tab_focus(args: &[String]) -> std::io::Result<i32> {
    let Some(raw_tab_id) = args.first() else {
        eprintln!("usage: herdr tab focus <tab_id>");
        return Ok(2);
    };
    if args.len() != 1 {
        eprintln!("usage: herdr tab focus <tab_id>");
        return Ok(2);
    }

    super::runtime::tab_focus(super::normalize_tab_id(raw_tab_id))
}

fn tab_rename(args: &[String]) -> std::io::Result<i32> {
    if args.len() < 2 {
        eprintln!("usage: herdr tab rename <tab_id> <label>");
        return Ok(2);
    }

    super::runtime::tab_rename(TabRenameParams {
        tab_id: super::normalize_tab_id(&args[0]),
        label: args[1..].join(" "),
    })
}

fn tab_move_to_workspace(args: &[String]) -> std::io::Result<i32> {
    match parse_tab_move_to_workspace_args(args) {
        Ok(params) => super::runtime::tab_move_to_workspace(params),
        Err(err) => {
            eprintln!("{err}");
            Ok(2)
        }
    }
}

fn parse_tab_move_to_workspace_args(args: &[String]) -> Result<TabMoveToWorkspaceParams, String> {
    let Some(raw_tab_id) = args.first().filter(|arg| !arg.starts_with('-')) else {
        return Err(tab_move_to_workspace_usage());
    };
    let mut new_workspace = false;
    let mut workspace_id = None;
    let mut label = None;
    let mut focus = true;

    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "--new-workspace" => {
                new_workspace = true;
                index += 1;
            }
            "--workspace" => {
                let Some(value) = args.get(index + 1) else {
                    return Err("missing value for --workspace".into());
                };
                workspace_id = Some(super::normalize_workspace_id(value));
                index += 2;
            }
            "--label" => {
                let Some(value) = args.get(index + 1) else {
                    return Err("missing value for --label".into());
                };
                label = Some(value.clone());
                index += 2;
            }
            "--focus" => {
                focus = true;
                index += 1;
            }
            "--no-focus" => {
                focus = false;
                index += 1;
            }
            other => return Err(format!("unknown option: {other}")),
        }
    }

    let destination = match (new_workspace, workspace_id) {
        (true, None) => TabMoveDestination::NewWorkspace { label },
        (false, Some(workspace_id)) if label.is_none() => {
            TabMoveDestination::Workspace { workspace_id }
        }
        _ => return Err(tab_move_to_workspace_usage()),
    };

    Ok(TabMoveToWorkspaceParams {
        tab_id: super::normalize_tab_id(raw_tab_id),
        destination,
        focus,
    })
}

fn tab_move_to_workspace_usage() -> String {
    "usage: herdr tab move-to-workspace <tab_id> --new-workspace [--label TEXT] [--focus|--no-focus]\n       herdr tab move-to-workspace <tab_id> --workspace <workspace_id> [--focus|--no-focus]".into()
}

fn tab_close(args: &[String]) -> std::io::Result<i32> {
    let Some(raw_tab_id) = args.first() else {
        eprintln!("usage: herdr tab close <tab_id>");
        return Ok(2);
    };
    if args.len() != 1 {
        eprintln!("usage: herdr tab close <tab_id>");
        return Ok(2);
    }

    super::runtime::tab_close(super::normalize_tab_id(raw_tab_id))
}

fn print_tab_help() {
    eprintln!("herdr tab commands:");
    eprintln!("  herdr tab list [--workspace <workspace_id>]");
    eprintln!(
        "  herdr tab create [--workspace <workspace_id>] [--cwd PATH] [--label TEXT] [--env KEY=VALUE] [--focus] [--no-focus]"
    );
    eprintln!("  herdr tab get <tab_id>");
    eprintln!("  herdr tab focus <tab_id>");
    eprintln!("  herdr tab rename <tab_id> <label>");
    eprintln!(
        "  herdr tab move-to-workspace <tab_id> --new-workspace [--label TEXT] [--focus] [--no-focus]"
    );
    eprintln!(
        "  herdr tab move-to-workspace <tab_id> --workspace <workspace_id> [--focus] [--no-focus]"
    );
    eprintln!("  herdr tab close <tab_id>");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn parse_tab_move_to_workspace_args_promotes_a_tab_into_a_named_space() {
        let params = parse_tab_move_to_workspace_args(&args(&[
            "issue:2",
            "--new-workspace",
            "--label",
            "logs",
        ]))
        .unwrap();

        assert_eq!(params.tab_id, "issue:2");
        assert!(params.focus);
        assert_eq!(
            params.destination,
            TabMoveDestination::NewWorkspace {
                label: Some("logs".into())
            }
        );
    }

    #[test]
    fn parse_tab_move_to_workspace_args_targets_an_existing_space() {
        let params = parse_tab_move_to_workspace_args(&args(&[
            "issue:2",
            "--workspace",
            "other",
            "--no-focus",
        ]))
        .unwrap();

        assert!(!params.focus);
        assert_eq!(
            params.destination,
            TabMoveDestination::Workspace {
                workspace_id: "other".into()
            }
        );
    }

    #[test]
    fn parse_tab_move_to_workspace_args_rejects_ambiguous_or_missing_destinations() {
        for arguments in [
            args(&["issue:2"]),
            args(&["issue:2", "--new-workspace", "--workspace", "other"]),
            args(&["issue:2", "--workspace", "other", "--label", "logs"]),
        ] {
            let err = parse_tab_move_to_workspace_args(&arguments).unwrap_err();
            assert!(err.contains("usage: herdr tab move-to-workspace"));
        }
    }
}
