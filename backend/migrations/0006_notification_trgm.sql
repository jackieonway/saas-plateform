-- 通知搜索性能优化: pg_trgm trigram GIN 索引 + 高频过滤复合索引
-- 幂等设计: 所有语句可重复执行 (IF NOT EXISTS / DO 块包裹), 服务重启时 migrate! 重复应用不报错;
-- 降级设计: CREATE EXTENSION 需要超管/贡献模块权限, 权限不足时 DO 块捕获异常仅 RAISE NOTICE 跳过,
--           不让迁移整体失败; GIN trgm 索引在扩展缺失时同样捕获跳过 (普通复合索引无外部依赖, 无条件成功)。

-- 1) pg_trgm 扩展: trigram 索引加速 LIKE/ILIKE '%...%' 的基础 (contrib 模块, 不改数据)
DO $$
BEGIN
    CREATE EXTENSION IF NOT EXISTS pg_trgm;
EXCEPTION WHEN OTHERS THEN
    RAISE NOTICE 'pg_trgm 扩展创建失败, 跳过 trgm 索引: %', SQLERRM;
END
$$;

-- 2) title trigram GIN 索引: 通知中心 q 搜索的 title ILIKE '%q%' 由 planner 自动选用 (SQL 无需改动)
DO $$
BEGIN
    CREATE INDEX IF NOT EXISTS idx_notifications_title_trgm
        ON notifications USING gin (title gin_trgm_ops);
EXCEPTION WHEN OTHERS THEN
    RAISE NOTICE 'title trgm GIN 索引创建失败 (pg_trgm 扩展可能未安装), 跳过: %', SQLERRM;
END
$$;

-- 3) content trigram GIN 索引: 同上, 服务于 content ILIKE '%q%'
DO $$
BEGIN
    CREATE INDEX IF NOT EXISTS idx_notifications_content_trgm
        ON notifications USING gin (content gin_trgm_ops);
EXCEPTION WHEN OTHERS THEN
    RAISE NOTICE 'content trgm GIN 索引创建失败 (pg_trgm 扩展可能未安装), 跳过: %', SQLERRM;
END
$$;

-- 4) 高频过滤复合索引: 「本人通知 + UNREAD 过滤 + 时间倒序」主路径
--    (list 的 filter=UNREAD 分页 与 unread-count 聚合; 含既有 (user_id, is_read) 索引的排序能力)
CREATE INDEX IF NOT EXISTS idx_notifications_user_read_created
    ON notifications (user_id, is_read, created_at DESC);
