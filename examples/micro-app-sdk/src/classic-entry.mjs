import * as sdk from './index.mjs'
import { installIframeBridge } from './iframe-bridge.js'

window.MicroSdk = Object.freeze({ ...sdk, sdkVersion: sdk.SDK_VERSION })
installIframeBridge(window)
