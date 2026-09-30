-- 令牌有效期/轮询间隔列改为 BIGINT, 与 Rust 结构体 i64 解码对齐
ALTER TABLE oauth_clients ALTER COLUMN access_token_validity TYPE BIGINT;
ALTER TABLE oauth_clients ALTER COLUMN refresh_token_validity TYPE BIGINT;
ALTER TABLE oauth_device_codes ALTER COLUMN interval_seconds TYPE BIGINT;
