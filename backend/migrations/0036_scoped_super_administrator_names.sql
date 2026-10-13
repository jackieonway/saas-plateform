-- Rename scoped administrator labels only. Preserve role IDs/codes and grants,
-- and never change users.is_super_admin (the platform-wide privilege flag).
UPDATE roles
SET name = '超级管理员',
    description = replace(replace(description, '租户管理员', '超级管理员'), '组织管理员', '超级管理员'),
    updated_at = now()
WHERE name IN ('租户管理员', '组织管理员');

UPDATE memberships
SET position = '超级管理员'
WHERE position IN ('租户管理员', '组织管理员');
