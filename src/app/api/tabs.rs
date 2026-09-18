use std::path::PathBuf;

use crate::api::schema::{
    EventData, EventEnvelope, EventKind, PaneInfo, ResponseResult, TabCreateParams, TabListParams,
    TabMoveDestination, TabMoveParams, TabMoveToWorkspaceParams, TabMoveToWorkspaceReason,
    TabMoveToWorkspaceResult, TabRenameParams, TabTarget,
};
use crate::app::{App, Mode};

use super::responses::{encode_error, encode_success};

impl App {
    pub(super) fn handle_tab_list(&mut self, id: String, params: TabListParams) -> String {
        let tabs = if let Some(workspace_id) = params.workspace_id {
            let Some(ws_idx) = self.parse_workspace_id(&workspace_id) else {
                return workspace_not_found(id, &workspace_id);
            };
            let Some(_) = self.state.workspaces.get(ws_idx) else {
                return workspace_not_found(id, &workspace_id);
            };
            self.tab_list_info(ws_idx)
        } else {
            let mut tabs = Vec::new();
            for (ws_idx, ws) in self.state.workspaces.iter().enumerate() {
                for tab_idx in 0..ws.tabs.len() {
                    if let Some(tab) = self.tab_info(ws_idx, tab_idx) {
                        tabs.push(tab);
                    }
                }
            }
            tabs
        };

        encode_success(id, ResponseResult::TabList { tabs })
    }

    pub(super) fn handle_tab_get(&mut self, id: String, target: TabTarget) -> String {
        let Some((ws_idx, tab_idx)) = self.parse_tab_id(&target.tab_id) else {
            return tab_not_found(id, &target.tab_id);
        };
        let Some(tab) = self.tab_info(ws_idx, tab_idx) else {
            return tab_not_found(id, &target.tab_id);
        };

        encode_success(id, ResponseResult::TabInfo { tab })
    }

    pub(super) fn handle_tab_create(&mut self, id: String, params: TabCreateParams) -> String {
        let TabCreateParams {
            workspace_id,
            cwd,
            focus,
            label,
            env,
        } = params;
        let ws_idx = if let Some(workspace_id) = workspace_id {
            let Some(ws_idx) = self.parse_workspace_id(&workspace_id) else {
                return workspace_not_found(id, &workspace_id);
            };
            ws_idx
        } else if let Some(active) = self.state.active {
            active
        } else {
            return encode_error(id, "workspace_not_found", "no active workspace");
        };
        let cwd = cwd.map(PathBuf::from).unwrap_or_else(|| {
            self.resolve_new_terminal_cwd(self.focused_pane_cwd_in_workspace(ws_idx))
        });
        let (rows, cols) = self.state.estimate_pane_size();
        let default_shell = self.state.default_shell.clone();
        let scrollback_limit_bytes = self.state.pane_scrollback_limit_bytes;
        let host_terminal_theme = self.state.host_terminal_theme;
        let host_terminal_appearance = self.state.host_terminal_appearance;
        let extra_env = match super::env::normalize_launch_env(env) {
            Ok(env) => env,
            Err((code, message)) => return encode_error(id, &code, message),
        };
        let result = self
            .state
            .workspaces
            .get_mut(ws_idx)
            .ok_or_else(|| std::io::Error::other("workspace disappeared"))
            .and_then(|ws| {
                ws.create_tab(
                    rows,
                    cols,
                    cwd,
                    scrollback_limit_bytes,
                    host_terminal_theme,
                    host_terminal_appearance,
                    crate::pane::PaneShellConfig::new(&default_shell, self.state.shell_mode),
                    extra_env,
                )
            });
        match result {
            Ok((tab_idx, terminal, runtime)) => {
                self.terminal_runtimes.insert(terminal.id.clone(), runtime);
                self.state.terminals.insert(terminal.id.clone(), terminal);
                self.state.remove_alias_shadowed_by_new_pane(
                    self.state.workspaces[ws_idx].tabs[tab_idx].root_pane,
                );
                if let Some(label) = label {
                    let workspace_id = self.state.workspaces[ws_idx].id.clone();
                    let tab_id = self.public_tab_id(ws_idx, tab_idx).unwrap_or_else(|| {
                        crate::workspace::public_tab_id_for_number(&workspace_id, tab_idx + 1)
                    });
                    if let Some(tab) = self
                        .state
                        .workspaces
                        .get_mut(ws_idx)
                        .and_then(|ws| ws.tabs.get_mut(tab_idx))
                    {
                        tab.set_custom_name(label);
                        crate::logging::tab_renamed(&workspace_id, &tab_id);
                    }
                }
                if focus {
                    self.state.switch_workspace_tab(ws_idx, tab_idx);
                    self.state.mode = Mode::Terminal;
                }
                self.schedule_session_save();
                self.emit_tab_created_events(ws_idx, tab_idx);
                encode_success(
                    id,
                    self.tab_created_result(ws_idx, tab_idx)
                        .expect("new tab should produce a complete create response"),
                )
            }
            Err(err) => encode_error(id, "tab_create_failed", err.to_string()),
        }
    }

    pub(super) fn handle_tab_focus(&mut self, id: String, target: TabTarget) -> String {
        let Some((ws_idx, tab_idx)) = self.parse_tab_id(&target.tab_id) else {
            return tab_not_found(id, &target.tab_id);
        };
        self.state.switch_workspace_tab(ws_idx, tab_idx);
        let tab = self.tab_info(ws_idx, tab_idx).unwrap();

        encode_success(id, ResponseResult::TabInfo { tab })
    }

    pub(super) fn handle_tab_rename(&mut self, id: String, params: TabRenameParams) -> String {
        let Some((ws_idx, tab_idx)) = self.parse_tab_id(&params.tab_id) else {
            return tab_not_found(id, &params.tab_id);
        };
        let workspace_id = self.state.workspaces[ws_idx].id.clone();
        let tab_id = self.public_tab_id(ws_idx, tab_idx).unwrap_or_else(|| {
            crate::workspace::public_tab_id_for_number(&workspace_id, tab_idx + 1)
        });
        let Some(tab) = self
            .state
            .workspaces
            .get_mut(ws_idx)
            .and_then(|ws| ws.tabs.get_mut(tab_idx))
        else {
            return tab_not_found(id, &params.tab_id);
        };
        tab.set_custom_name(params.label.clone());
        crate::logging::tab_renamed(&workspace_id, &tab_id);
        self.schedule_session_save();
        self.emit_event(EventEnvelope {
            event: EventKind::TabRenamed,
            data: EventData::TabRenamed {
                tab_id: self.public_tab_id(ws_idx, tab_idx).unwrap(),
                workspace_id: self.public_workspace_id(ws_idx),
                label: params.label,
            },
        });
        let tab = self.tab_info(ws_idx, tab_idx).unwrap();

        encode_success(id, ResponseResult::TabInfo { tab })
    }

    pub(super) fn handle_tab_move(&mut self, id: String, params: TabMoveParams) -> String {
        let Some((ws_idx, tab_idx)) = self.parse_tab_id(&params.tab_id) else {
            return tab_not_found(id, &params.tab_id);
        };
        let Some(ws) = self.state.workspaces.get(ws_idx) else {
            return tab_not_found(id, &params.tab_id);
        };
        if params.insert_index > ws.tabs.len() {
            return encode_error(
                id,
                "tab_move_failed",
                format!("insert_index {} is out of bounds", params.insert_index),
            );
        }

        let tab_id = self
            .public_tab_id(ws_idx, tab_idx)
            .unwrap_or_else(|| crate::workspace::public_tab_id_for_number(&ws.id, tab_idx + 1));
        let workspace_id = self.public_workspace_id(ws_idx);
        let insert_index = params.insert_index;
        let moved = self
            .state
            .workspaces
            .get_mut(ws_idx)
            .is_some_and(|ws| ws.move_tab(tab_idx, insert_index));
        let tabs = self.tab_list_info(ws_idx);
        if moved {
            self.schedule_session_save();
            self.emit_event(EventEnvelope {
                event: EventKind::TabMoved,
                data: EventData::TabMoved {
                    tab_id,
                    workspace_id,
                    insert_index,
                    tabs: tabs.clone(),
                },
            });
        }

        encode_success(id, ResponseResult::TabList { tabs })
    }

    pub(super) fn handle_tab_move_to_workspace(
        &mut self,
        id: String,
        params: TabMoveToWorkspaceParams,
    ) -> String {
        let Some((source_ws_idx, tab_idx)) = self.parse_tab_id(&params.tab_id) else {
            return tab_not_found(id, &params.tab_id);
        };
        let Some(previous_tab_id) = self.public_tab_id(source_ws_idx, tab_idx) else {
            return tab_not_found(id, &params.tab_id);
        };
        let previous_workspace_id = self.public_workspace_id(source_ws_idx);

        let destination_ws_idx = match &params.destination {
            TabMoveDestination::Workspace { workspace_id } => {
                // parse_workspace_id's numeric fallback does not bounds-check.
                let Some(ws_idx) = self
                    .parse_workspace_id(workspace_id)
                    .filter(|ws_idx| self.state.workspaces.get(*ws_idx).is_some())
                else {
                    return workspace_not_found(id, workspace_id);
                };
                Some(ws_idx)
            }
            TabMoveDestination::NewWorkspace { .. } => None,
        };

        if destination_ws_idx == Some(source_ws_idx) {
            return self.unchanged_tab_move(
                id,
                source_ws_idx,
                tab_idx,
                previous_workspace_id,
                previous_tab_id,
                TabMoveToWorkspaceReason::SameWorkspace,
            );
        }
        if self.state.workspaces[source_ws_idx].tabs.len() <= 1 {
            return self.unchanged_tab_move(
                id,
                source_ws_idx,
                tab_idx,
                previous_workspace_id,
                previous_tab_id,
                TabMoveToWorkspaceReason::OnlyTab,
            );
        }

        let pane_ids = self.state.workspaces[source_ws_idx].tabs[tab_idx]
            .layout
            .pane_ids();
        let previous_pane_ids = pane_ids
            .iter()
            .filter_map(|pane_id| {
                self.public_pane_id(source_ws_idx, *pane_id)
                    .map(|public_id| (public_id, *pane_id))
            })
            .collect::<Vec<_>>();
        let identity_cwd = self.tab_identity_cwd(source_ws_idx, tab_idx);
        let tab_label = self.state.workspaces[source_ws_idx].tabs[tab_idx]
            .custom_name
            .clone();
        let previous_focus = self.state.current_pane_focus_target();

        let Some(taken) = self.state.workspaces[source_ws_idx].take_tab_for_move(tab_idx) else {
            return encode_error(
                id,
                "tab_move_failed",
                format!("tab {} could not be moved", params.tab_id),
            );
        };

        let (target_ws_idx, target_tab_idx, created_workspace) = match destination_ws_idx {
            Some(target_ws_idx) => {
                let target_tab_idx = self.state.workspaces[target_ws_idx].insert_moved_tab(taken);
                (target_ws_idx, target_tab_idx, false)
            }
            None => {
                let label = match params.destination {
                    TabMoveDestination::NewWorkspace { label } => label,
                    TabMoveDestination::Workspace { .. } => None,
                };
                let workspace = crate::workspace::Workspace::from_existing_tab(
                    label.or(tab_label),
                    identity_cwd,
                    taken,
                );
                let insert_idx = self.workspace_group_insert_index(source_ws_idx);
                self.state.workspaces.insert(insert_idx, workspace);
                if let Some(active) = self.state.active.filter(|active| *active >= insert_idx) {
                    self.state.active = Some(active + 1);
                }
                if self.state.selected >= insert_idx {
                    self.state.selected += 1;
                }
                (insert_idx, 0, true)
            }
        };

        for (public_id, pane_id) in previous_pane_ids {
            self.state.public_pane_id_aliases.insert(public_id, pane_id);
        }
        let target_workspace_id = self.public_workspace_id(target_ws_idx);
        self.state
            .retarget_pane_workspace_references(&pane_ids, &target_workspace_id);
        // Detaching the tab shifted the source's later tabs, and inserting a space shifted
        // every later workspace, so anything holding an index has to be re-resolved.
        self.resync_runtime_pane_indices();
        // Detaching the tab shifted the source's later tabs, and inserting a space shifted
        // every later workspace, so anything holding an index has to be re-resolved.
        for pane_id in &pane_ids {
            self.state.remove_alias_shadowed_by_new_pane(*pane_id);
        }
        // The focus captured before the move names the workspace the pane has just left.
        let previous_focus = previous_focus.map(|mut focus| {
            if pane_ids.contains(&focus.pane_id) {
                focus.workspace_id = target_workspace_id.clone();
            }
            focus
        });

        if params.focus {
            self.state
                .switch_workspace_tab(target_ws_idx, target_tab_idx);
            let focused = self.state.workspaces[target_ws_idx].tabs[target_tab_idx]
                .layout
                .focused();
            self.state
                .record_pane_focus_change(previous_focus, target_ws_idx, focused);
            self.state.mode = Mode::Terminal;
        }

        self.state.mark_session_dirty();
        self.schedule_session_save();

        let Some(tab) = self.tab_info(target_ws_idx, target_tab_idx) else {
            return encode_error(id, "tab_move_failed", "moved tab is unavailable");
        };
        let panes = self.tab_pane_info(target_ws_idx, target_tab_idx);
        let created_workspace = created_workspace.then(|| self.workspace_info(target_ws_idx));

        if let Some(workspace) = &created_workspace {
            self.emit_event(EventEnvelope {
                event: EventKind::WorkspaceCreated,
                data: EventData::WorkspaceCreated {
                    workspace: workspace.clone(),
                },
            });
        }
        self.emit_event(EventEnvelope {
            event: EventKind::TabCreated,
            data: EventData::TabCreated { tab: tab.clone() },
        });
        self.emit_event(EventEnvelope {
            event: EventKind::TabClosed,
            data: EventData::TabClosed {
                tab_id: previous_tab_id.clone(),
                workspace_id: previous_workspace_id.clone(),
            },
        });
        if let Some(layout) = self.pane_layout_snapshot(target_ws_idx, target_tab_idx) {
            self.emit_layout_updated_snapshot(layout);
        }

        encode_success(
            id,
            ResponseResult::TabMovedToWorkspace {
                move_result: Box::new(TabMoveToWorkspaceResult {
                    changed: true,
                    reason: None,
                    previous_workspace_id,
                    previous_tab_id,
                    tab,
                    panes,
                    created_workspace,
                }),
            },
        )
    }

    fn unchanged_tab_move(
        &self,
        id: String,
        ws_idx: usize,
        tab_idx: usize,
        previous_workspace_id: String,
        previous_tab_id: String,
        reason: TabMoveToWorkspaceReason,
    ) -> String {
        let Some(tab) = self.tab_info(ws_idx, tab_idx) else {
            return tab_not_found(id, &previous_tab_id);
        };
        let panes = self.tab_pane_info(ws_idx, tab_idx);

        encode_success(
            id,
            ResponseResult::TabMovedToWorkspace {
                move_result: Box::new(TabMoveToWorkspaceResult {
                    changed: false,
                    reason: Some(reason),
                    previous_workspace_id,
                    previous_tab_id,
                    tab,
                    panes,
                    created_workspace: None,
                }),
            },
        )
    }

    fn tab_pane_info(&self, ws_idx: usize, tab_idx: usize) -> Vec<PaneInfo> {
        self.state
            .workspaces
            .get(ws_idx)
            .and_then(|ws| ws.tabs.get(tab_idx))
            .map(|tab| tab.layout.pane_ids())
            .unwrap_or_default()
            .into_iter()
            .filter_map(|pane_id| self.pane_info(ws_idx, pane_id))
            .collect()
    }

    /// Where a space promoted out of `ws_idx` belongs. The sidebar renders a worktree
    /// group as one contiguous block, so landing inside that block would read as after
    /// the whole group anyway.
    fn workspace_group_insert_index(&self, ws_idx: usize) -> usize {
        let Some(key) = self
            .state
            .workspaces
            .get(ws_idx)
            .and_then(|workspace| workspace.worktree_space())
            .map(|space| space.key.clone())
        else {
            return ws_idx + 1;
        };
        self.state
            .workspaces
            .iter()
            .enumerate()
            .filter(|(_, workspace)| {
                workspace
                    .worktree_space()
                    .is_some_and(|space| space.key == key)
            })
            .map(|(idx, _)| idx + 1)
            .max()
            .unwrap_or(ws_idx + 1)
            .max(ws_idx + 1)
    }

    fn tab_identity_cwd(&self, ws_idx: usize, tab_idx: usize) -> PathBuf {
        self.state
            .workspaces
            .get(ws_idx)
            .and_then(|ws| ws.tabs.get(tab_idx))
            .and_then(|tab| tab.terminal_id(tab.layout.focused()))
            .and_then(|terminal_id| self.state.terminals.get(terminal_id))
            .map(|terminal| terminal.cwd.clone())
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| "/".into()))
    }

    pub(super) fn handle_tab_close(&mut self, id: String, target: TabTarget) -> String {
        let Some((ws_idx, tab_idx)) = self.parse_tab_id(&target.tab_id) else {
            return tab_not_found(id, &target.tab_id);
        };
        let Some(tab_id) = self.public_tab_id(ws_idx, tab_idx) else {
            return tab_not_found(id, &target.tab_id);
        };
        let workspace_id = self.public_workspace_id(ws_idx);
        let Some(ws) = self.state.workspaces.get(ws_idx) else {
            return tab_not_found(id, &target.tab_id);
        };
        let closes_workspace = ws.tabs.len() <= 1;
        let terminal_ids = self.state.terminal_ids_for_tab(ws_idx, tab_idx);
        let pane_ids = ws
            .tabs
            .get(tab_idx)
            .map(|tab| tab.layout.pane_ids())
            .unwrap_or_default();

        if closes_workspace {
            if self.state.confirm_implicit_worktree_group_close(ws_idx) {
                return encode_error(
                    id,
                    "confirmation_required",
                    "closing this tab would close a worktree group",
                );
            }
            let workspace = self.workspace_info(ws_idx);
            self.state.selected = ws_idx;
            self.state.close_selected_workspace();
            self.state.remove_plugin_pane_records(pane_ids);
            self.shutdown_detached_terminal_runtimes();
            self.emit_event(EventEnvelope {
                event: EventKind::TabClosed,
                data: EventData::TabClosed {
                    tab_id,
                    workspace_id: workspace_id.clone(),
                },
            });
            self.emit_event(EventEnvelope {
                event: EventKind::WorkspaceClosed,
                data: EventData::WorkspaceClosed {
                    workspace_id,
                    workspace: Some(workspace),
                },
            });
            return encode_success(id, ResponseResult::Ok {});
        }

        let Some(ws) = self.state.workspaces.get_mut(ws_idx) else {
            return tab_not_found(id, &target.tab_id);
        };
        if !ws.close_tab(tab_idx) {
            return encode_error(
                id,
                "tab_close_failed",
                format!("tab {} could not be closed", target.tab_id),
            );
        }
        self.state.remove_plugin_pane_records(pane_ids);
        self.state.remove_unattached_terminal_ids(terminal_ids);
        self.shutdown_detached_terminal_runtimes();
        self.schedule_session_save();
        self.emit_event(EventEnvelope {
            event: EventKind::TabClosed,
            data: EventData::TabClosed {
                tab_id,
                workspace_id,
            },
        });

        encode_success(id, ResponseResult::Ok {})
    }

    fn tab_list_info(&self, ws_idx: usize) -> Vec<crate::api::schema::TabInfo> {
        self.state
            .workspaces
            .get(ws_idx)
            .map(|ws| {
                (0..ws.tabs.len())
                    .filter_map(|idx| self.tab_info(ws_idx, idx))
                    .collect()
            })
            .unwrap_or_default()
    }
}

fn workspace_not_found(id: String, workspace_id: &str) -> String {
    encode_error(
        id,
        "workspace_not_found",
        format!("workspace {workspace_id} not found"),
    )
}

fn tab_not_found(id: String, tab_id: &str) -> String {
    encode_error(id, "tab_not_found", format!("tab {tab_id} not found"))
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{exiting_test_command, shutdown_test_runtimes};
    use super::*;
    use crate::{
        api::schema::SuccessResponse,
        config::{Config, ShellModeConfig},
        workspace::Workspace,
    };

    #[test]
    fn api_tab_close_last_tab_closes_workspace_and_emits_both_events() {
        let event_hub = crate::api::EventHub::default();
        let (_api_tx, api_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(
            &Config::default(),
            crate::app::AppPolicy::TEST,
            None,
            api_rx,
            event_hub.clone(),
        );
        app.state.workspaces = vec![Workspace::test_new("tabs")];
        app.state.active = Some(0);
        app.state.selected = 0;
        let tab_id = app.public_tab_id(0, 0).unwrap();
        let workspace_id = app.public_workspace_id(0);

        let response = app.handle_tab_close(
            "req".into(),
            TabTarget {
                tab_id: tab_id.clone(),
            },
        );

        let success: SuccessResponse = serde_json::from_str(&response).unwrap();
        assert_eq!(success.result, ResponseResult::Ok {});
        assert!(app.state.workspaces.is_empty());
        assert!(app.state.active.is_none());
        let events = event_hub.events_after(0);
        assert_eq!(
            events
                .iter()
                .map(|(_, event)| event.event)
                .collect::<Vec<_>>(),
            [EventKind::TabClosed, EventKind::WorkspaceClosed]
        );
        assert!(matches!(
            &events[0].1.data,
            EventData::TabClosed {
                tab_id: closed_tab_id,
                workspace_id: closed_workspace_id,
            } if closed_tab_id == &tab_id && closed_workspace_id == &workspace_id
        ));
        assert!(matches!(
            &events[1].1.data,
            EventData::WorkspaceClosed {
                workspace_id: closed_workspace_id,
                workspace: Some(workspace),
            } if closed_workspace_id == &workspace_id
                && workspace.workspace_id == workspace_id
        ));
    }

    #[test]
    fn api_tab_move_reorders_tabs_in_target_workspace() {
        let event_hub = crate::api::EventHub::default();
        let (_api_tx, api_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(
            &Config::default(),
            crate::app::AppPolicy::TEST,
            None,
            api_rx,
            event_hub.clone(),
        );
        let mut workspace = Workspace::test_new("tabs");
        workspace.test_add_tab(Some("two"));
        workspace.test_add_tab(Some("three"));
        app.state.workspaces = vec![workspace];
        app.state.active = Some(0);
        app.state.selected = 0;
        let moved_root = app.state.workspaces[0].tabs[0].root_pane;
        let moved_id = app.public_tab_id(0, 0).unwrap();

        let response = app.handle_tab_move(
            "req".into(),
            TabMoveParams {
                tab_id: moved_id.clone(),
                insert_index: 3,
            },
        );

        let success: SuccessResponse = serde_json::from_str(&response).unwrap();
        let ResponseResult::TabList { tabs } = success.result else {
            panic!("expected tab list");
        };
        assert_eq!(app.state.workspaces[0].tabs[2].root_pane, moved_root);
        assert_eq!(tabs[2].tab_id, app.public_tab_id(0, 2).unwrap());
        let events = event_hub.events_after(0);
        assert!(events.iter().any(|(_, event)| {
            matches!(
                &event.data,
                EventData::TabMoved {
                    tab_id,
                    workspace_id,
                    insert_index: 3,
                    tabs,
                } if tab_id == &moved_id
                    && workspace_id == &app.public_workspace_id(0)
                    && tabs[2].tab_id == moved_id
            )
        }));
    }

    fn test_app(event_hub: crate::api::EventHub) -> App {
        let (_api_tx, api_rx) = tokio::sync::mpsc::unbounded_channel();
        App::new(
            &Config::default(),
            crate::app::AppPolicy::TEST,
            None,
            api_rx,
            event_hub,
        )
    }

    /// Source workspace with two tabs, the second holding two panes.
    fn app_with_splittable_second_tab(event_hub: crate::api::EventHub) -> App {
        let mut app = test_app(event_hub);
        let mut workspace = Workspace::test_new("source");
        workspace.test_add_tab(Some("side"));
        workspace.switch_tab(1);
        workspace.test_split(ratatui::layout::Direction::Horizontal);
        workspace.switch_tab(0);
        app.state.workspaces = vec![workspace];
        app.state.active = Some(0);
        app.state.selected = 0;
        app.state.ensure_test_terminals();
        app
    }

    fn move_result(response: &str) -> TabMoveToWorkspaceResult {
        let success: SuccessResponse = serde_json::from_str(response).unwrap();
        let ResponseResult::TabMovedToWorkspace { move_result } = success.result else {
            panic!("expected tab move result");
        };
        *move_result
    }

    #[test]
    fn tab_move_to_new_workspace_carries_every_pane_and_renumbers_ids() {
        let event_hub = crate::api::EventHub::default();
        let mut app = app_with_splittable_second_tab(event_hub.clone());
        let moved_tab_id = app.public_tab_id(0, 1).unwrap();
        let moved_panes = app.state.workspaces[0].tabs[1].layout.pane_ids();
        assert_eq!(moved_panes.len(), 2);

        let response = app.handle_tab_move_to_workspace(
            "req".into(),
            TabMoveToWorkspaceParams {
                tab_id: moved_tab_id.clone(),
                destination: TabMoveDestination::NewWorkspace { label: None },
                focus: true,
            },
        );

        let result = move_result(&response);
        assert!(result.changed);
        assert_eq!(result.reason, None);
        assert_eq!(result.previous_tab_id, moved_tab_id);
        assert_eq!(result.panes.len(), 2);
        assert_eq!(app.state.workspaces.len(), 2);
        assert_eq!(app.state.workspaces[0].tabs.len(), 1);
        assert_eq!(app.state.workspaces[1].tabs.len(), 1);
        assert_eq!(
            app.state.workspaces[1].tabs[0].layout.pane_ids(),
            moved_panes
        );
        assert_eq!(app.state.workspaces[1].tabs[0].number, 1);
        for (index, pane_id) in moved_panes.iter().enumerate() {
            assert_eq!(
                app.state.workspaces[1].public_pane_number(*pane_id),
                Some(index + 1)
            );
            assert_eq!(app.state.workspaces[0].public_pane_number(*pane_id), None);
        }
        // The promoted space inherits the tab's own name.
        assert_eq!(app.state.workspaces[1].custom_name.as_deref(), Some("side"));
        assert!(app.state.workspaces[1].worktree_space.is_none());
        assert_eq!(
            result.created_workspace.map(|ws| ws.workspace_id),
            Some(app.public_workspace_id(1))
        );
        app.state.assert_invariants_for_test();
    }

    #[test]
    fn tab_move_keeps_previous_pane_ids_resolvable() {
        let event_hub = crate::api::EventHub::default();
        let mut app = app_with_splittable_second_tab(event_hub);
        let moved_tab_id = app.public_tab_id(0, 1).unwrap();
        let moved_panes = app.state.workspaces[0].tabs[1].layout.pane_ids();
        let previous_pane_ids = moved_panes
            .iter()
            .map(|pane_id| app.public_pane_id(0, *pane_id).unwrap())
            .collect::<Vec<_>>();

        app.handle_tab_move_to_workspace(
            "req".into(),
            TabMoveToWorkspaceParams {
                tab_id: moved_tab_id,
                destination: TabMoveDestination::NewWorkspace { label: None },
                focus: false,
            },
        );

        for (previous_id, pane_id) in previous_pane_ids.iter().zip(&moved_panes) {
            assert_eq!(app.parse_pane_id(previous_id), Some((1, *pane_id)));
            assert_ne!(app.public_pane_id(1, *pane_id).as_ref(), Some(previous_id));
        }
        app.state.assert_invariants_for_test();
    }

    #[test]
    fn tab_move_refuses_the_only_tab_in_a_workspace() {
        let event_hub = crate::api::EventHub::default();
        let mut app = test_app(event_hub.clone());
        app.state.workspaces = vec![Workspace::test_new("only")];
        app.state.active = Some(0);
        app.state.ensure_test_terminals();
        let tab_id = app.public_tab_id(0, 0).unwrap();

        let response = app.handle_tab_move_to_workspace(
            "req".into(),
            TabMoveToWorkspaceParams {
                tab_id: tab_id.clone(),
                destination: TabMoveDestination::NewWorkspace { label: None },
                focus: true,
            },
        );

        let result = move_result(&response);
        assert!(!result.changed);
        assert_eq!(result.reason, Some(TabMoveToWorkspaceReason::OnlyTab));
        assert_eq!(result.tab.tab_id, tab_id);
        assert_eq!(app.state.workspaces.len(), 1);
        assert!(event_hub.events_after(0).is_empty());
        app.state.assert_invariants_for_test();
    }

    #[test]
    fn tab_move_rejects_a_workspace_id_that_is_out_of_range() {
        let event_hub = crate::api::EventHub::default();
        let mut app = app_with_splittable_second_tab(event_hub.clone());
        let tab_id = app.public_tab_id(0, 1).unwrap();

        let response = app.handle_tab_move_to_workspace(
            "req".into(),
            TabMoveToWorkspaceParams {
                tab_id,
                destination: TabMoveDestination::Workspace {
                    workspace_id: "w_99".into(),
                },
                focus: true,
            },
        );

        let error: crate::api::schema::ErrorResponse = serde_json::from_str(&response).unwrap();
        assert_eq!(error.error.code, "workspace_not_found");
        assert_eq!(app.state.workspaces.len(), 1);
        assert_eq!(app.state.workspaces[0].tabs.len(), 2);
        assert!(event_hub.events_after(0).is_empty());
        app.state.assert_invariants_for_test();
    }

    #[test]
    fn tab_move_to_its_own_workspace_changes_nothing() {
        let event_hub = crate::api::EventHub::default();
        let mut app = app_with_splittable_second_tab(event_hub.clone());
        let tab_id = app.public_tab_id(0, 1).unwrap();
        let workspace_id = app.public_workspace_id(0);

        let response = app.handle_tab_move_to_workspace(
            "req".into(),
            TabMoveToWorkspaceParams {
                tab_id,
                destination: TabMoveDestination::Workspace { workspace_id },
                focus: true,
            },
        );

        let result = move_result(&response);
        assert!(!result.changed);
        assert_eq!(result.reason, Some(TabMoveToWorkspaceReason::SameWorkspace));
        assert_eq!(app.state.workspaces.len(), 1);
        assert_eq!(app.state.workspaces[0].tabs.len(), 2);
        assert!(event_hub.events_after(0).is_empty());
    }

    #[test]
    fn tab_move_to_existing_workspace_retargets_workspace_scoped_references() {
        let event_hub = crate::api::EventHub::default();
        let mut app = app_with_splittable_second_tab(event_hub);
        app.state.workspaces.push(Workspace::test_new("target"));
        app.state.ensure_test_terminals();
        let source_workspace_id = app.public_workspace_id(0);
        let target_workspace_id = app.public_workspace_id(1);
        let moved_tab_id = app.public_tab_id(0, 1).unwrap();
        let moved_panes = app.state.workspaces[0].tabs[1].layout.pane_ids();
        let focus_pane = moved_panes[0];
        app.state.previous_pane_focus = Some(crate::app::state::PaneFocusTarget {
            workspace_id: source_workspace_id.clone(),
            pane_id: focus_pane,
        });
        app.state.toast = Some(crate::app::state::ToastNotification {
            kind: crate::app::state::ToastKind::Finished,
            title: "done".into(),
            context: String::new(),
            position: None,
            target: Some(crate::app::state::ToastTarget {
                workspace_id: source_workspace_id.clone(),
                pane_id: focus_pane,
            }),
        });
        app.state.pending_agent_notifications.insert(
            focus_pane,
            crate::app::state::PendingAgentNotification {
                pane_id: focus_pane,
                workspace_id: source_workspace_id,
                agent_label: "agent".into(),
                known_agent: None,
                kind: crate::app::state::ToastKind::Finished,
                state: crate::detect::AgentState::Idle,
                deadline: std::time::Instant::now(),
            },
        );

        let response = app.handle_tab_move_to_workspace(
            "req".into(),
            TabMoveToWorkspaceParams {
                tab_id: moved_tab_id,
                destination: TabMoveDestination::Workspace {
                    workspace_id: target_workspace_id.clone(),
                },
                focus: false,
            },
        );

        assert!(move_result(&response).changed);
        assert_eq!(app.state.workspaces[1].tabs.len(), 2);
        assert_eq!(
            app.state.previous_pane_focus.as_ref().unwrap().workspace_id,
            target_workspace_id
        );
        assert_eq!(
            app.state
                .toast
                .as_ref()
                .unwrap()
                .target
                .as_ref()
                .unwrap()
                .workspace_id,
            target_workspace_id
        );
        assert_eq!(
            app.state.pending_agent_notifications[&focus_pane].workspace_id,
            target_workspace_id
        );
        app.state.assert_invariants_for_test();
    }

    #[test]
    fn promoted_space_lands_after_its_source_and_keeps_selection() {
        let event_hub = crate::api::EventHub::default();
        let mut app = app_with_splittable_second_tab(event_hub.clone());
        app.state.workspaces.push(Workspace::test_new("later"));
        app.state.ensure_test_terminals();
        app.state.active = Some(1);
        app.state.selected = 1;
        let later_workspace_id = app.public_workspace_id(1);
        let moved_tab_id = app.public_tab_id(0, 1).unwrap();

        app.handle_tab_move_to_workspace(
            "req".into(),
            TabMoveToWorkspaceParams {
                tab_id: moved_tab_id,
                destination: TabMoveDestination::NewWorkspace {
                    label: Some("promoted".into()),
                },
                focus: false,
            },
        );

        assert_eq!(app.state.workspaces.len(), 3);
        assert_eq!(
            app.state.workspaces[1].custom_name.as_deref(),
            Some("promoted")
        );
        assert_eq!(app.public_workspace_id(2), later_workspace_id);
        assert_eq!(app.state.active, Some(2));
        assert_eq!(app.state.selected, 2);
        let kinds = event_hub
            .events_after(0)
            .iter()
            .map(|(_, event)| event.event)
            .collect::<Vec<_>>();
        assert_eq!(
            kinds,
            [
                EventKind::WorkspaceCreated,
                EventKind::TabCreated,
                EventKind::TabClosed,
                EventKind::LayoutUpdated,
            ]
        );
        app.state.assert_invariants_for_test();
    }

    #[test]
    fn moving_the_focused_tab_leaves_no_stale_focus_record() {
        let event_hub = crate::api::EventHub::default();
        let mut app = app_with_splittable_second_tab(event_hub);
        app.state.workspaces[0].switch_tab(1);
        let focused_pane = app.state.workspaces[0].tabs[1].layout.focused();
        let moved_tab_id = app.public_tab_id(0, 1).unwrap();

        let response = app.handle_tab_move_to_workspace(
            "req".into(),
            TabMoveToWorkspaceParams {
                tab_id: moved_tab_id,
                destination: TabMoveDestination::NewWorkspace { label: None },
                focus: true,
            },
        );

        assert!(move_result(&response).changed);
        assert_eq!(app.state.active, Some(1));
        assert!(app.state.workspaces[1].pane_state(focused_pane).is_some());
        // The recorded previous focus must still live in the workspace it names.
        app.state.assert_invariants_for_test();
    }

    #[test]
    fn promoting_a_grouped_tab_lands_after_the_whole_worktree_group() {
        fn membership(is_linked_worktree: bool) -> crate::workspace::WorktreeSpaceMembership {
            crate::workspace::WorktreeSpaceMembership {
                key: "repo-key".into(),
                label: "herdr".into(),
                repo_root: "/repo/herdr".into(),
                checkout_path: "/repo/herdr".into(),
                is_linked_worktree,
            }
        }

        let event_hub = crate::api::EventHub::default();
        let mut app = app_with_splittable_second_tab(event_hub);
        app.state.workspaces.push(Workspace::test_new("child"));
        app.state.workspaces.push(Workspace::test_new("unrelated"));
        app.state.ensure_test_terminals();
        app.state.workspaces[0].worktree_space = Some(membership(false));
        app.state.workspaces[1].worktree_space = Some(membership(true));
        let child_id = app.public_workspace_id(1);
        let unrelated_id = app.public_workspace_id(2);
        let moved_tab_id = app.public_tab_id(0, 1).unwrap();

        app.handle_tab_move_to_workspace(
            "req".into(),
            TabMoveToWorkspaceParams {
                tab_id: moved_tab_id,
                destination: TabMoveDestination::NewWorkspace { label: None },
                focus: false,
            },
        );

        // The sidebar renders the group as one block, so the new space follows the block.
        assert_eq!(app.public_workspace_id(1), child_id);
        assert_eq!(app.public_workspace_id(3), unrelated_id);
        assert!(app.state.workspaces[2].worktree_space.is_none());
        app.state.assert_invariants_for_test();
    }

    #[test]
    fn promoting_a_tab_follows_overlay_and_focus_bookkeeping() {
        let event_hub = crate::api::EventHub::default();
        let mut app = app_with_splittable_second_tab(event_hub);
        // A third tab after the moved one, so the source's tab indices shift too.
        app.state.workspaces[0].test_add_tab(Some("trailing"));
        app.state.ensure_test_terminals();
        let moved_pane = app.state.workspaces[0].tabs[1].layout.focused();
        let trailing_pane = app.state.workspaces[0].tabs[2].root_pane;
        app.test_track_overlay_pane(moved_pane, 0, 1);
        app.test_track_overlay_pane(trailing_pane, 0, 2);
        app.last_focus = Some((0, moved_pane));
        let moved_tab_id = app.public_tab_id(0, 1).unwrap();

        app.handle_tab_move_to_workspace(
            "req".into(),
            TabMoveToWorkspaceParams {
                tab_id: moved_tab_id,
                destination: TabMoveDestination::NewWorkspace { label: None },
                focus: false,
            },
        );

        // The moved overlay followed its pane into the promoted space.
        let moved_overlay = app.test_overlay_indices(moved_pane).expect("moved overlay");
        assert_eq!(moved_overlay, (1, 0));
        // The trailing tab slid down one slot in the source space.
        let trailing_overlay = app
            .test_overlay_indices(trailing_pane)
            .expect("trailing overlay");
        assert_eq!(trailing_overlay, (0, 1));
        assert_eq!(app.last_focus, Some((1, moved_pane)));
        app.state.assert_invariants_for_test();
    }

    #[test]
    fn promoting_a_tab_keeps_runtime_workspace_indices_aligned() {
        let event_hub = crate::api::EventHub::default();
        let mut app = app_with_splittable_second_tab(event_hub);
        app.state.workspaces.push(Workspace::test_new("later"));
        app.state.ensure_test_terminals();
        let later_pane = app.state.workspaces[1].tabs[0].root_pane;
        app.last_focus = Some((1, later_pane));
        let moved_tab_id = app.public_tab_id(0, 1).unwrap();

        app.handle_tab_move_to_workspace(
            "req".into(),
            TabMoveToWorkspaceParams {
                tab_id: moved_tab_id,
                destination: TabMoveDestination::NewWorkspace { label: None },
                focus: false,
            },
        );

        // "later" shifted from index 1 to 2, and the stored focus index must follow it.
        let (focus_ws_idx, focus_pane) = app.last_focus.expect("runtime focus");
        assert_eq!(focus_pane, later_pane);
        assert_eq!(focus_ws_idx, 2);
        assert!(app.state.workspaces[focus_ws_idx]
            .pane_state(later_pane)
            .is_some());
    }

    #[test]
    fn tab_move_holds_identity_invariants_on_adversarial_state() {
        let event_hub = crate::api::EventHub::default();
        let mut app = test_app(event_hub);
        app.state = crate::app::AppState::test_with_adversarial_identity_state();
        app.state.ensure_test_terminals();
        let moved_tab_id = app.public_tab_id(0, 1).unwrap();
        let moved_panes = app.state.workspaces[0].tabs[1].layout.pane_ids();
        let previous_pane_ids = moved_panes
            .iter()
            .map(|pane_id| app.public_pane_id(0, *pane_id).unwrap())
            .collect::<Vec<_>>();

        let response = app.handle_tab_move_to_workspace(
            "req".into(),
            TabMoveToWorkspaceParams {
                tab_id: moved_tab_id,
                destination: TabMoveDestination::NewWorkspace { label: None },
                focus: true,
            },
        );

        assert!(move_result(&response).changed);
        app.state.assert_invariants_for_test();
        for (previous_id, pane_id) in previous_pane_ids.iter().zip(&moved_panes) {
            assert_eq!(app.parse_pane_id(previous_id), Some((1, *pane_id)));
        }
    }

    #[tokio::test]
    async fn tab_create_follows_cached_focused_pane_cwd_without_runtime() {
        let event_hub = crate::api::EventHub::default();
        let (_api_tx, api_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(
            &Config::default(),
            crate::app::AppPolicy::TEST,
            None,
            api_rx,
            event_hub,
        );
        app.state.default_shell = exiting_test_command().into();
        app.state.shell_mode = ShellModeConfig::NonLogin;
        let workspace = Workspace::test_new("tabs");
        let focused_pane = workspace.tabs[0].root_pane;
        app.state.workspaces = vec![workspace];
        app.state.active = Some(0);
        app.state.selected = 0;
        app.state.ensure_test_terminals();
        let cached_cwd = std::env::temp_dir();
        let terminal_id = app.state.workspaces[0]
            .terminal_id(focused_pane)
            .cloned()
            .unwrap();
        app.state.terminals.get_mut(&terminal_id).unwrap().cwd = cached_cwd.clone();

        let response = app.handle_tab_create(
            "req".into(),
            TabCreateParams {
                workspace_id: None,
                cwd: None,
                focus: false,
                label: None,
                env: Default::default(),
            },
        );

        let success: SuccessResponse = serde_json::from_str(&response).unwrap();
        assert!(matches!(success.result, ResponseResult::TabCreated { .. }));
        let created = &app.state.workspaces[0].tabs[1];
        let created_terminal_id = created.terminal_id(created.root_pane).unwrap();
        let created_cwd = &app.state.terminals.get(created_terminal_id).unwrap().cwd;
        assert_eq!(
            crate::worktree::canonical_or_original(created_cwd),
            crate::worktree::canonical_or_original(&cached_cwd)
        );
        shutdown_test_runtimes(&mut app);
    }
}
