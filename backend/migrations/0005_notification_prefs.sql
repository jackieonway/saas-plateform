-- 通知偏好: 免打扰时段 (DND) + 静音通知类型 (个人维度, 登录即可读写自己的)
CREATE TABLE IF NOT EXISTS notification_prefs (
  user_id UUID PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
  dnd_enabled BOOLEAN NOT NULL DEFAULT false,
  dnd_start VARCHAR(5) NOT NULL DEFAULT '22:00',
  dnd_end VARCHAR(5) NOT NULL DEFAULT '07:00',
  muted_types TEXT NOT NULL DEFAULT '',
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
