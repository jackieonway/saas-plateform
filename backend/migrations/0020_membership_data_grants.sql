-- 数据范围独立于角色和菜单，绑定用户在某组织的直接成员关系。
CREATE TABLE membership_data_grants (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    membership_id UUID NOT NULL REFERENCES memberships(id) ON DELETE CASCADE,
    application_id UUID NOT NULL REFERENCES applications(id) ON DELETE CASCADE,
    resource_code VARCHAR(100) NOT NULL,
    action_code VARCHAR(100) NOT NULL,
    scope_type VARCHAR(20) NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT ck_membership_data_grants_scope
        CHECK (scope_type IN ('SELF', 'ORG', 'ORG_SUBTREE', 'TENANT')),
    CONSTRAINT ck_membership_data_grants_resource
        CHECK (length(trim(resource_code)) > 0 AND length(trim(action_code)) > 0),
    CONSTRAINT uq_membership_data_grants
        UNIQUE (membership_id, application_id, resource_code, action_code)
);

CREATE INDEX idx_membership_data_grants_context
    ON membership_data_grants (membership_id, application_id);
