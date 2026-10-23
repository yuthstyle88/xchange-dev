-- Forward migration (#4995, design docs/design/three-decimal-currencies.md §5):
-- Rewrites custody JSON amounts from integer hundredths to decimal strings with two places.

UPDATE tx_drafts
SET data = jsonb_set(
    data,
    '{amount}',
    to_jsonb(((data->>'amount')::numeric * 0.01)::text)
)
WHERE jsonb_typeof(data->'amount') = 'number'
   OR (jsonb_typeof(data->'amount') = 'string' AND (data->>'amount') ~ '^-?[0-9]+$');

UPDATE ops_state
SET target = jsonb_set(
    target,
    '{amounts}',
    COALESCE(
        (
            SELECT jsonb_object_agg(
                e.k,
                CASE
                    WHEN jsonb_typeof(e.v) = 'number'
                      OR (jsonb_typeof(e.v) = 'string' AND (e.v #>> '{}') ~ '^-?[0-9]+$')
                    THEN to_jsonb(((e.v #>> '{}')::numeric * 0.01)::text)
                    ELSE e.v
                END
            )
            FROM jsonb_each(target->'amounts') AS e(k, v)
        ),
        '{}'::jsonb
    )
)
WHERE jsonb_typeof(target->'amounts') = 'object';

UPDATE cash_exceptions
SET data = (
    SELECT jsonb_object_agg(
        top.k,
        CASE
            WHEN top.k IN ('targetDifference', 'cashVariance', 'safeVariance')
             AND jsonb_typeof(top.v) = 'object'
            THEN COALESCE(
                (
                    SELECT jsonb_object_agg(
                        cur.k,
                        CASE
                            WHEN jsonb_typeof(cur.v) = 'number'
                              OR (jsonb_typeof(cur.v) = 'string' AND (cur.v #>> '{}') ~ '^-?[0-9]+$')
                            THEN to_jsonb(((cur.v #>> '{}')::numeric * 0.01)::text)
                            ELSE cur.v
                        END
                    )
                    FROM jsonb_each(top.v) AS cur(k, v)
                ),
                '{}'::jsonb
            )
            ELSE top.v
        END
    )
    FROM jsonb_each(data) AS top(k, v)
)
WHERE jsonb_typeof(data->'targetDifference') = 'object'
   OR jsonb_typeof(data->'cashVariance') = 'object'
   OR jsonb_typeof(data->'safeVariance') = 'object';
