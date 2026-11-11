-- #6286: the OTP channels a customer may choose, set by the owner (owner-approved one column, 2026-11-05).
-- NULL = not set: the default email, then SMS. Existing rows stay NULL, so nothing changes until the owner saves.
ALTER TABLE customer_app_settings
    ADD COLUMN otp_channels text[] NULL
    CONSTRAINT customer_app_settings_otp_channels_check CHECK (
        otp_channels IS NULL OR (
            array_ndims(otp_channels) = 1
            AND array_lower(otp_channels, 1) = 1
            AND cardinality(otp_channels) BETWEEN 1 AND 2
            AND otp_channels <@ ARRAY['email', 'sms']::text[]
            AND (cardinality(otp_channels) = 1 OR otp_channels[1] <> otp_channels[2])
        )
    );
