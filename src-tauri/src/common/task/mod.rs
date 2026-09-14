/*
 * task module -- background task execution system.
 *
 * Responsibilities:
 *  - task_types:   shared type definitions for the task system
 *  - task_manager: global task registry and cancellation token management
 *  - task_logger:  global log broadcaster that pushes backend logs to the frontend UI
 *
 * Design philosophy:
 *  All frontend-to-backend commands are executed through this system.
 *  Tasks run in background threads so they do not block the frontend UI.
 *  Logs are appended to the frontend log panel in real time via a persistent Channel.
 *  Users can cancel all running tasks through the stop button in the UI.
 *
 * Follows Soul.md conventions:
 *  - organized as struct + impl, each .rs file contains one utility class
 *  - file names use snake_case
 */

pub mod task_context;
pub mod task_logger;
pub mod task_manager;
pub mod task_runner;
pub mod task_types;
