/*
 * scene_types.rs - Scene Hierarchy related shared types.
 *
 * Split from types.rs, follows Soul.md conventions: each .rs contains related types only.
 */

use serde::Serialize;

/// Scene hierarchy tree node
#[derive(Debug, Clone, Serialize)]
pub struct SceneNode {
    /// Node name (GameObject.m_Name)
    pub name: String,
    /// This GameObject's path_id
    pub path_id: i64,
    /// This GameObject's class_id
    pub class_id: i32,
    /// Child nodes
    pub children: Vec<SceneNode>,
    /// Component refs (MeshFilter etc.) as path_id -> class_name
    pub components: Vec<ComponentRef>,
}

/// Component reference summary
#[derive(Debug, Clone, Serialize)]
pub struct ComponentRef {
    pub path_id: i64,
    pub class_name: String,
    pub class_id: i32,
}

/// Scene hierarchy tree + diagnostic info
#[derive(Debug, Clone, Serialize)]
pub struct SceneHierarchyResult {
    pub roots: Vec<SceneNode>,
    pub diagnostics: Vec<String>,
}
