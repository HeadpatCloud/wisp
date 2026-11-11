// Wisp draws its own title bar on Windows only; macOS and Linux keep the native one.
export const customTitleBar = navigator.userAgent.includes('Windows')
export const isMac = navigator.userAgent.includes('Mac')
