export const tenantTypes = ["PERSONAL", "COMPANY", "PUBLIC_INSTITUTION", "GOVERNMENT", "OTHER"] as const
export type TenantType = (typeof tenantTypes)[number]

export const tenantTypeLabels: Record<TenantType, string> = {
  PERSONAL: "个人",
  COMPANY: "公司",
  PUBLIC_INSTITUTION: "事业单位",
  GOVERNMENT: "政府",
  OTHER: "其他",
}

export const organizationTypes = ["COMPANY", "SUBSIDIARY", "BRANCH", "PUBLIC_INSTITUTION", "GOVERNMENT", "OTHER"] as const
export type OrganizationType = (typeof organizationTypes)[number]

export const organizationTypeLabels: Record<OrganizationType, string> = {
  COMPANY: "公司",
  SUBSIDIARY: "子公司",
  BRANCH: "分公司",
  PUBLIC_INSTITUTION: "事业单位",
  GOVERNMENT: "政府",
  OTHER: "其他",
}
