import { zodResolver } from "@hookform/resolvers/zod"
import type { FieldValues, Resolver } from "react-hook-form"
import type { ZodType } from "zod"

/**
 * RHF + zod v4 的 resolver 适配:
 * z.coerce.* 会使 z.input 与 z.output 不一致,而页面表单类型按输出类型声明,
 * 此处统一以输出类型为准(resolver 运行时会输出转换后的值)。
 */
export function zResolver<T extends FieldValues>(schema: ZodType<T, any, any>): Resolver<T, any, T> {
  return zodResolver(schema) as unknown as Resolver<T, any, T>
}
