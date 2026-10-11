CREATE TABLE region_versions (
  version VARCHAR(20) PRIMARY KEY,
  label VARCHAR(100) NOT NULL,
  source_version VARCHAR(20),
  imported BOOLEAN NOT NULL DEFAULT false,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
COMMENT ON TABLE region_versions IS '地区数据版本';
COMMENT ON COLUMN region_versions.version IS '版本标识，如2023';
COMMENT ON COLUMN region_versions.label IS '版本名称，如2023版';
COMMENT ON COLUMN region_versions.source_version IS '复制来源版本';
COMMENT ON COLUMN region_versions.imported IS '是否已完成内置数据导入';
COMMENT ON COLUMN region_versions.created_at IS '创建时间';
COMMENT ON COLUMN region_versions.updated_at IS '更新时间';

CREATE TABLE regions (
  version VARCHAR(20) NOT NULL REFERENCES region_versions(version) ON DELETE CASCADE,
  source_id VARCHAR(32) NOT NULL,
  code VARCHAR(12) NOT NULL,
  parent_code VARCHAR(12) NOT NULL DEFAULT '0',
  name VARCHAR(128) NOT NULL,
  name_pinyin VARCHAR(512) NOT NULL,
  name_short VARCHAR(64) NOT NULL,
  level INT NOT NULL DEFAULT 1,
  type VARCHAR(12),
  type_name VARCHAR(32),
  post_code VARCHAR(512),
  source_url VARCHAR(512),
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY(version, source_id)
);
CREATE INDEX idx_regions_parent ON regions(version, parent_code, code);
CREATE INDEX idx_regions_code ON regions(version, code);
CREATE INDEX idx_regions_name ON regions(version, name);
COMMENT ON TABLE regions IS '按版本维护的地区记录，编码在原始数据中不唯一';
COMMENT ON COLUMN regions.version IS '地区数据版本';
COMMENT ON COLUMN regions.source_id IS '原始主键或新建记录主键';
COMMENT ON COLUMN regions.code IS '地区编码';
COMMENT ON COLUMN regions.parent_code IS '父级编码';
COMMENT ON COLUMN regions.name IS '地区名称';
COMMENT ON COLUMN regions.name_pinyin IS '地区拼音';
COMMENT ON COLUMN regions.name_short IS '地区拼音简称';
COMMENT ON COLUMN regions.level IS '层级';
COMMENT ON COLUMN regions.type IS '地区类型';
COMMENT ON COLUMN regions.type_name IS '地区类型名称 111:主城区 112:城乡结合区 121:镇中心区 122:镇乡结合区 123:特殊区域 210:乡中心区 220:村庄';
COMMENT ON COLUMN regions.post_code IS '邮编';
COMMENT ON COLUMN regions.source_url IS '获取地区链接';
COMMENT ON COLUMN regions.created_at IS '创建时间';
COMMENT ON COLUMN regions.updated_at IS '更新时间';
