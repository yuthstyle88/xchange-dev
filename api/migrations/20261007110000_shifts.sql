CREATE TABLE shifts (
    id               text        PRIMARY KEY,
    seq              bigint      GENERATED ALWAYS AS IDENTITY UNIQUE,
    branch_id        text        NOT NULL REFERENCES branches (id),
    status           text        NOT NULL CHECK (status IN ('OPEN', 'CLOSED')),
    opened_at        timestamptz NOT NULL,
    opened_by        text        NOT NULL,
    closed_at        timestamptz,
    closed_by        text,
    close_note       text        NOT NULL DEFAULT '',
    open_request_id  text        NOT NULL,
    close_request_id text,
    UNIQUE (branch_id, open_request_id),
    UNIQUE (branch_id, close_request_id)
);
CREATE UNIQUE INDEX shifts_one_open_per_branch ON shifts (branch_id) WHERE status = 'OPEN';

CREATE TABLE shift_balances (
    shift_id   text          NOT NULL REFERENCES shifts (id),
    code       char(3)       NOT NULL,
    opening    numeric(20,2) NOT NULL DEFAULT 0 CHECK (opening >= 0),
    expected   numeric(20,2) NOT NULL DEFAULT 0 CHECK (expected >= 0),
    counted    numeric(20,2) CHECK (counted >= 0),
    difference numeric(20,2),
    PRIMARY KEY (shift_id, code)
);

CREATE TABLE stock_movements (
    id         text          PRIMARY KEY,
    seq        bigint        GENERATED ALWAYS AS IDENTITY UNIQUE,
    branch_id  text          NOT NULL REFERENCES branches (id),
    shift_id   text          NOT NULL REFERENCES shifts (id),
    code       char(3)       NOT NULL,
    direction  text          NOT NULL CHECK (direction IN ('IN', 'OUT')),
    amount     numeric(20,2) NOT NULL CHECK (amount > 0),
    reason     text          NOT NULL,
    request_id text          NOT NULL,
    at         timestamptz   NOT NULL,
    by         text          NOT NULL,
    UNIQUE (branch_id, request_id)
);
CREATE INDEX stock_movements_shift ON stock_movements (shift_id);
