import js from "@eslint/js"
import { defineConfig } from "eslint/config"
import tseslint from "typescript-eslint"
import reactHooks from "eslint-plugin-react-hooks"

export default defineConfig([
  {
    files: ["src/**/*.{ts,tsx}", "vite.config.ts"],
    extends: [js.configs.recommended, tseslint.configs.recommended],
    plugins: { "react-hooks": reactHooks },
    rules: {
      // Phase 1 exposes existing typing/cleanup debt without rewriting business code.
      // Keep these diagnostics visible; tsconfig also leaves noUnused* disabled.
      "@typescript-eslint/no-unused-vars": "warn",
      "@typescript-eslint/no-explicit-any": "warn",
      "prefer-const": "warn",
      "no-useless-assignment": "warn",
      "preserve-caught-error": "warn",
      "react-hooks/rules-of-hooks": "error",
      "react-hooks/exhaustive-deps": "warn",
    },
  },
  {
    files: [
      "src/components/micro/iframe.ts",
      "src/lib/notification-protocol.ts",
      "src/pages/system/settings/PlatformSettingsPage.tsx",
      "vite.config.ts",
    ],
    // These input validators intentionally reject ASCII control characters.
    // The regex is a security boundary, not an accidental literal character.
    rules: { "no-control-regex": "off" },
  },
])
