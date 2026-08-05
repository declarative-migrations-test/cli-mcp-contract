CREATE SCHEMA app;

CREATE TABLE app.accounts (
    id text PRIMARY KEY,
    email text NOT NULL UNIQUE,
    created_at timestamptz NOT NULL DEFAULT current_timestamp
);

CREATE INDEX accounts_created_at_idx ON app.accounts (created_at);
