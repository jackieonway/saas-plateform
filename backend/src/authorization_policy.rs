//! Stable authorization decisions; callers must authenticate before using them.
use uuid::Uuid;

/// A confidential caller may inspect only tokens belonging to its registered app.
/// Client identifiers and context hashes alone do not prove authentication.
pub fn application_binding(
    caller_application: Uuid,
    token_application: Uuid,
    requested_code: &str,
    audience: &str,
) -> bool {
    caller_application == token_application && requested_code == audience
}

/// Existing member-directory data scopes, independent of business ownership.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemberScope {
    /// Assignment candidates remain in the current organization.
    SelfScope,
    /// Assignment candidates remain in the current organization.
    Org,
    /// The explicitly granted organization subtree.
    OrgSubtree,
    /// Tenant scope still intersects the caller's hard ceiling.
    Tenant,
}

/// Distinguish omitted legacy ceiling, explicit tenant scope and a finite list.
pub enum OrganizationCeiling<'a> {
    /// Legacy omission is limited to the selected organization.
    Current,
    /// Trusted platform context explicitly permits tenant-wide access.
    Tenant,
    /// A validated finite ceiling; it must contain the selected organization.
    Finite(&'a [Uuid]),
}

/// Intersect a member lookup's grant with an independent organization ceiling.
/// Tenant identity and registered resource/action checks belong to the use case;
/// selecting or assigning a member never changes a business record's SELF owner.
pub fn member_organization_allowed(
    scope: MemberScope,
    grant: &[Uuid],
    ceiling: OrganizationCeiling<'_>,
    current: Uuid,
    target: Uuid,
) -> bool {
    let hard = match ceiling {
        OrganizationCeiling::Current => target == current,
        OrganizationCeiling::Tenant => true,
        OrganizationCeiling::Finite(ids) => ids.contains(&current) && ids.contains(&target),
    };
    let granted = match scope {
        MemberScope::SelfScope => {
            (grant.is_empty() || grant.contains(&current)) && target == current
        }
        MemberScope::Org => grant == [current] && target == current,
        MemberScope::OrgSubtree => grant.contains(&current) && grant.contains(&target),
        MemberScope::Tenant => grant.is_empty(),
    };
    hard && granted
}
