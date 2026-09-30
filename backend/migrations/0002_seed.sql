-- 种子数据 (租户/应用/菜单/组织/部门/用户/角色/授权链/OAuth 客户端) 由 Rust 在启动时写入:
--   见 backend/src/seed.rs — 仅当 users 表为空时执行, 幂等; 用户密码使用 Argon2 现算,
--   OAuth client secret 存 SHA-256 哈希。此文件仅作为迁移规划占位。
SELECT 1;
