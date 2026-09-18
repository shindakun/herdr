use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::common::AgentStatus;
use super::panes::PaneInfo;
use super::workspaces::WorkspaceInfo;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TabCreateParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default)]
    pub focus: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub env: HashMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, Default)]
pub struct TabListParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TabRenameParams {
    pub tab_id: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TabMoveParams {
    pub tab_id: String,
    pub insert_index: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TabInfo {
    pub tab_id: String,
    pub workspace_id: String,
    pub number: usize,
    pub label: String,
    pub focused: bool,
    pub pane_count: usize,
    pub agent_status: AgentStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TabMoveToWorkspaceParams {
    pub tab_id: String,
    pub destination: TabMoveDestination,
    #[serde(default)]
    pub focus: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TabMoveDestination {
    Workspace {
        workspace_id: String,
    },
    NewWorkspace {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TabMoveToWorkspaceResult {
    pub changed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<TabMoveToWorkspaceReason>,
    pub previous_workspace_id: String,
    pub previous_tab_id: String,
    pub tab: TabInfo,
    pub panes: Vec<PaneInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_workspace: Option<WorkspaceInfo>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TabMoveToWorkspaceReason {
    /// The tab is the only tab in its workspace, so moving it would empty that workspace.
    OnlyTab,
    /// The destination workspace already owns the tab.
    SameWorkspace,
}
