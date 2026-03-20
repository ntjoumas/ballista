export interface LauncherInfo {
  launcher_version: string
}

export interface Connection {
  address: string
  heapSize: string
  icon: string
  iconDataUrl?: string | null
  id: string
  javaHome: string
  javaArgs: string
  name: string
  username: string
  password: string
  verify: boolean
  group: string
  environment: string
  notes: string
  donotcache: boolean
  lastConnected: number | null
  groupOrder: number
  environmentOrder: number
  sortOrder: number
  showConsole: boolean

  // the below properties are transient and are used only in the UI
  nodeId: string
  parentId: string
}

export interface UntrustedCert {
  der?: string
  subject?: string
  issuer?: string
  expires_on?: string,
  sha256sum: string,
}
