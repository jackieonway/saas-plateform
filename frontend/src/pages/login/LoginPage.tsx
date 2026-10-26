import * as React from "react"
import { useNavigate, useSearchParams } from "react-router-dom"
import { captureSessionFence, useAuthStore } from "@/stores/auth"
import { logoutPlatformSession, switchPlatformContext } from "@/lib/api"
import { safePlatformReturn, startUnifiedLogin } from "@/lib/authentication-mode"
import BffLoginPage from "./BffLoginPage"

export default function LoginPage() {
  const session = useAuthStore(state => state.bffSession)
  const token = useAuthStore(state => state.token)
  const reconcile = useAuthStore(state => state.reconcileSession)
  const navigate = useNavigate()
  const [params] = useSearchParams()
  const returnTo = safePlatformReturn(params.get("return"))

  React.useEffect(() => {
    if (token) navigate(returnTo, { replace: true })
  }, [token, returnTo, navigate])

  return <BffLoginPage session={session} returnTo={returnTo}
    onLogin={() => startUnifiedLogin(returnTo)}
    onReload={reconcile}
    onSelect={async member => {
      const expected = captureSessionFence()
      await switchPlatformContext(member.organizationId, member.membershipId, expected)
      // Only the committed store token drives the effect above. In-flight or
      // superseded selections cannot navigate from an unmounted login form.
    }}
    onLogout={async scope => {
      if (await logoutPlatformSession(captureSessionFence(), scope)) navigate("/login?signedOut=1", { replace: true })
    }} />
}
