import { useFormContext } from "react-hook-form"
import { FormControl, FormField, FormItem, FormLabel, FormMessage } from "@/components/ui/form"
import { Input } from "@/components/ui/input"

export function ContactFields({ creating }: { creating: boolean }) {
  const { control } = useFormContext<{ contactName: string; contactPhone: string; contactEmail: string }>()
  return <div className="space-y-3 border-t pt-4">
    <p className="text-sm font-medium">联系人信息</p>
    <FormField control={control} name="contactName" render={({ field }) => <FormItem>
      <FormLabel>联系人姓名{creating && <span className="text-destructive"> *</span>}</FormLabel>
      <FormControl><Input autoComplete="name" placeholder="联系人姓名" {...field} /></FormControl><FormMessage />
    </FormItem>} />
    <FormField control={control} name="contactPhone" render={({ field }) => <FormItem>
      <FormLabel>联系人电话</FormLabel><FormControl><Input type="tel" autoComplete="tel" placeholder="手机或固定电话" {...field} /></FormControl><FormMessage />
    </FormItem>} />
    <FormField control={control} name="contactEmail" render={({ field }) => <FormItem>
      <FormLabel>联系人 Email</FormLabel><FormControl><Input type="email" autoComplete="email" placeholder="name@example.com" {...field} /></FormControl><FormMessage />
    </FormItem>} />
    <p className="text-xs text-muted-foreground">{creating
      ? "电话或 Email 至少填写一项。自动开通首位管理员，电话优先作为用户名，两者均可登录；新账号初始密码创建后显示。"
      : "维护业务联系人；管理员账号资料与授权可在用户管理、角色管理中维护。"}</p>
  </div>
}
