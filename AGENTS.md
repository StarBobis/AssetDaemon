# Coding Standards
- If you need to run scripts or replace content in files, the use of any PowerShell scripts is strictly prohibited; otherwise, encoding issues are guaranteed to occur.
- Inserting any Chinese characters or Chinese comments into the code is strictly prohibited.

# Build Map Architecture
- The goal is to build the index (i.e., the SQLite database) only once, enabling extremely fast relationship queries during subsequent usage.
- The Build Map process must not be too slow, and the build process should maximize CPU utilization as much as possible.
- SQLite does not store relationships as specific fields. Its purpose is to persist the CPU time invested in parsing so that the next time a bundle needs to be parsed, the previous parsing results can be reused directly by reading from the database, rather than treating it as a traditional database with numerous fields.

# Task & Logging System
- Every task must notify the Task Manager upon allocation.
- Every task must be stoppable via the terminate button in the log area.
- No task shall block the frontend UI or cause any UI lag/stuttering.
- Each task must produce detailed logs suitable for debugging and performance analysis. However, logs should not be overly verbose to the point of hindering manual analysis; they should contain only essential information.

# 流程

- 每次修改后记得提交并推送