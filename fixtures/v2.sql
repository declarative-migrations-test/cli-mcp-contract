CREATE SCHEMA app;

CREATE TABLE app.accounts (
    id text PRIMARY KEY,
    email text NOT NULL UNIQUE,
    status text NOT NULL DEFAULT 'active',
    created_at timestamptz NOT NULL DEFAULT current_timestamp,
    updated_at timestamptz NOT NULL DEFAULT current_timestamp,
    CONSTRAINT accounts_status_check CHECK (status IN ('active', 'disabled'))
);

CREATE INDEX accounts_created_at_idx ON app.accounts (created_at);
CREATE INDEX accounts_status_idx ON app.accounts (status) WHERE status = 'active';

CREATE TABLE app.account_events (
    event_id text PRIMARY KEY,
    account_id text NOT NULL REFERENCES app.accounts(id) ON DELETE CASCADE,
    event_type text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT current_timestamp
);

CREATE INDEX account_events_account_idx
    ON app.account_events (account_id, created_at);
