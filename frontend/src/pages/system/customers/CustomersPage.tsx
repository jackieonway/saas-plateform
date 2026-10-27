import { useAuthStore } from "@/stores/auth"
import { UserDirectory } from "../users/UsersPage"

export default function CustomersPage() {
  const namespace = useAuthStore(state => state.contextKey)
  return <UserDirectory key={`${namespace}:CUSTOMER`} memberType="CUSTOMER" />
}
