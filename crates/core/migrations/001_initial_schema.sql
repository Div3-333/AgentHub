CREATE TABLE IF NOT EXISTS graphs (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    definition JSON NOT NULL
);

CREATE TABLE IF NOT EXISTS executions (
    id TEXT PRIMARY KEY NOT NULL,
    graph_id TEXT NOT NULL,
    status TEXT NOT NULL,
    started_at INTEGER NOT NULL,
    completed_at INTEGER
);

CREATE TABLE IF NOT EXISTS node_logs (
    id TEXT PRIMARY KEY NOT NULL,
    exec_id TEXT NOT NULL,
    node_id TEXT NOT NULL,
    inputs JSON,
    outputs JSON,
    stdout TEXT
);
