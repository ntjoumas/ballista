<script setup lang="ts">
import type { Connection } from "~/types"
import { LandingScreenServerStatus } from "~/enums"
import { Channel, invoke } from "@tauri-apps/api/core"
import { fetch as tauriFetch } from "@tauri-apps/plugin-http"
import { ask, open, save as saveDialog } from "@tauri-apps/plugin-dialog"
import { open as shellOpen } from "@tauri-apps/plugin-shell"

type SortMode = "group" | "name" | "lastConnected" | "status"
type EnvironmentBucket = {
  key: string
  label: string
  order: number
  servers: Connection[]
}
type GroupBucket = {
  name: string
  order: number
  environments: EnvironmentBucket[]
}
type ImportDraft = Connection & {
  importKey: string
  selected: boolean
}
type ExportDraft = Connection & {
  exportKey: string
  selected: boolean
}
type BulkJavaDraft = Connection & {
  bulkKey: string
  selected: boolean
}
type DragState =
  | { kind: "group", groupName: string }
  | { kind: "environment", groupName: string, environmentKey: string }
  | { kind: "server", groupName: string, environmentKey: string, serverId: string }
type DropTarget =
  | { kind: "group", groupName: string }
  | { kind: "environment", groupName: string, environmentKey: string }
  | { kind: "server", groupName: string, environmentKey: string, serverId: string }

const isLoading = ref<boolean>(false)
const isInitializing = ref<boolean>(true)
const initializationError = ref<string | null>(null)
const progressMessage = ref<string>("Connecting...")
const launchError = ref<string | null>(null)
const actionMessage = ref<string | null>(null)
const searchFilter = ref<string>("")
const selectedServerId = ref<string | null>(null)
const sortBy = ref<SortMode>((localStorage.getItem("launcher-sort") as SortMode) || "group")
const showSortMenu = ref(false)
const isPersistingOrder = ref(false)
const dragging = ref<DragState | null>(null)
const dropTarget = ref<DropTarget | null>(null)
const showImportModal = ref(false)
const isImporting = ref(false)
const importSourceLabel = ref("")
const importDrafts = ref<ImportDraft[]>([])
const bulkImportUsername = ref("")
const bulkImportPassword = ref("")
const bulkImportJavaHome = ref("")
const showExportModal = ref(false)
const isExporting = ref(false)
const exportDrafts = ref<ExportDraft[]>([])
const showBulkJavaModal = ref(false)
const isApplyingBulkJava = ref(false)
const bulkJavaDrafts = ref<BulkJavaDraft[]>([])
const bulkJavaHomeValue = ref("")

const isGroupTarget = (groupName: string) =>
  dropTarget.value?.kind === "group" && dropTarget.value.groupName === groupName

const isEnvironmentTarget = (groupName: string, environmentKey: string) =>
  dropTarget.value?.kind === "environment"
  && dropTarget.value.groupName === groupName
  && dropTarget.value.environmentKey === environmentKey

const isServerTarget = (groupName: string, environmentKey: string, serverId: string) =>
  dropTarget.value?.kind === "server"
  && dropTarget.value.groupName === groupName
  && dropTarget.value.environmentKey === environmentKey
  && dropTarget.value.serverId === serverId

watch(sortBy, (v) => {
  localStorage.setItem("launcher-sort", v)
  showSortMenu.value = false
})

const servers = ref<Connection[]>([])
const allImportSelected = computed(() =>
  importDrafts.value.length > 0 && importDrafts.value.every((draft) => draft.selected),
)
const selectedImportCount = computed(() =>
  importDrafts.value.filter((draft) => draft.selected).length,
)
const allExportSelected = computed(() =>
  exportDrafts.value.length > 0 && exportDrafts.value.every((draft) => draft.selected),
)
const selectedExportCount = computed(() =>
  exportDrafts.value.filter((draft) => draft.selected).length,
)
const allBulkJavaSelected = computed(() =>
  bulkJavaDrafts.value.length > 0 && bulkJavaDrafts.value.every((draft) => draft.selected),
)
const selectedBulkJavaCount = computed(() =>
  bulkJavaDrafts.value.filter((draft) => draft.selected).length,
)

const normalizeImportDraft = (draft: Partial<Connection>, index: number): ImportDraft => ({
  address: draft.address ?? "",
  heapSize: draft.heapSize ?? "512m",
  icon: draft.icon ?? "",
  iconDataUrl: draft.iconDataUrl ?? null,
  id: draft.id ?? "",
  javaHome: draft.javaHome ?? "",
  javaArgs: draft.javaArgs ?? "",
  name: draft.name ?? `Imported Server ${index + 1}`,
  username: typeof draft.username === "string" ? draft.username : "",
  password: typeof draft.password === "string" ? draft.password : "",
  verify: draft.verify ?? true,
  group: draft.group ?? "Default",
  environment: draft.environment ?? "",
  notes: draft.notes ?? "",
  donotcache: draft.donotcache ?? false,
  lastConnected: draft.lastConnected ?? null,
  groupOrder: draft.groupOrder ?? 0,
  environmentOrder: draft.environmentOrder ?? 0,
  sortOrder: draft.sortOrder ?? 0,
  showConsole: draft.showConsole ?? false,
  nodeId: draft.nodeId ?? "",
  parentId: draft.parentId ?? "",
  importKey: `${draft.id ?? "import"}-${index}`,
  selected: true,
})

const normalizeExportDraft = (server: Connection, selected: boolean): ExportDraft => ({
  ...server,
  exportKey: server.id,
  selected,
})

const normalizeBulkJavaDraft = (server: Connection, selected: boolean): BulkJavaDraft => ({
  ...server,
  bulkKey: server.id,
  selected,
})

const sortByManualOrder = (a: Connection, b: Connection) =>
  (a.sortOrder ?? 0) - (b.sortOrder ?? 0) || a.name.localeCompare(b.name)
const sortByGroupOrder = (a: GroupBucket, b: GroupBucket) =>
  a.order - b.order || a.name.localeCompare(b.name)
const sortByEnvironmentOrder = (a: EnvironmentBucket, b: EnvironmentBucket) =>
  a.order - b.order || a.label.localeCompare(b.label)

// Connectivity status tracking (lifted from BriefServerInfo)
const serverStatuses = reactive<Record<string, LandingScreenServerStatus>>({})

const checkConnectivity = async (server: Connection) => {
  serverStatuses[server.id] = LandingScreenServerStatus.PENDING
  try {
    await tauriFetch(`${server.address}/api/system/info`, {
      method: "GET",
      danger: { acceptInvalidCerts: true, acceptInvalidHostnames: true },
      connectTimeout: 2000,
      headers: { "X-Requested-With": "Ballista" },
    })
    serverStatuses[server.id] = LandingScreenServerStatus.AVAILABLE
  } catch {
    serverStatuses[server.id] = LandingScreenServerStatus.UNAVAILABLE
  }
}

const loadConnections = async () => {
  isInitializing.value = true
  initializationError.value = null

  try {
    const response = await invoke<string>("load_connections")
    servers.value = JSON.parse(response)
    servers.value.forEach(checkConnectivity)
  } catch (e) {
    initializationError.value = `Failed to load connections: ${e}`
  } finally {
    isInitializing.value = false
  }
}

onMounted(loadConnections)
onMounted(() => {
  window.addEventListener("mouseup", finishReorder)
})
onUnmounted(() => {
  window.removeEventListener("mouseup", finishReorder)
})

const filteredServers = computed(() =>
  servers.value.filter((server) => {
    const search = searchFilter.value.toLowerCase()
    if (!search.length) return true
    const name = server.name.toLowerCase()
    const url = server.address.toLowerCase()
    return name.includes(search) || url.includes(search)
  }),
)

const sortedServers = computed(() => {
  const list = [...filteredServers.value]
  switch (sortBy.value) {
    case "name":
      return list.sort((a, b) => a.name.localeCompare(b.name))
    case "lastConnected":
      return list.sort((a, b) => (b.lastConnected ?? 0) - (a.lastConnected ?? 0))
    case "status":
      return list.sort((a, b) => {
        const order = { [LandingScreenServerStatus.AVAILABLE]: 0, [LandingScreenServerStatus.PENDING]: 1, [LandingScreenServerStatus.UNAVAILABLE]: 2 }
        return (order[serverStatuses[a.id] ?? 2] ?? 2) - (order[serverStatuses[b.id] ?? 2] ?? 2)
      })
    default:
      return list.sort(sortByManualOrder)
  }
})

const isGrouped = computed(() => sortBy.value === "group")

const groupedServers = computed<GroupBucket[]>(() => {
  const groups = new Map<string, { order: number, environments: Map<string, EnvironmentBucket> }>()

  for (const server of filteredServers.value) {
    const groupName = server.group?.trim() || "Ungrouped"
    const environmentName = server.environment?.trim() || ""
    const environmentKey = environmentName || "__default__"

    if (!groups.has(groupName)) {
      groups.set(groupName, {
        order: server.groupOrder ?? 0,
        environments: new Map<string, EnvironmentBucket>(),
      })
    }

    const group = groups.get(groupName)!
    group.order = Math.min(group.order, server.groupOrder ?? group.order)

    if (!group.environments.has(environmentKey)) {
      group.environments.set(environmentKey, {
        key: environmentKey,
        label: environmentName || "General",
        order: server.environmentOrder ?? 0,
        servers: [],
      })
    }

    const environment = group.environments.get(environmentKey)!
    environment.order = Math.min(environment.order, server.environmentOrder ?? environment.order)
    environment.servers.push(server)
  }

  return [...groups.entries()]
    .map(([name, group]) => ({
      name,
      order: group.order,
      environments: [...group.environments.values()]
        .map((environment) => ({
          ...environment,
          servers: [...environment.servers].sort(sortByManualOrder),
        }))
        .sort(sortByEnvironmentOrder),
    }))
    .sort(sortByGroupOrder)
})

const hasNamedEnvironment = (environment: EnvironmentBucket) =>
  environment.key !== "__default__"

const collapsedGroups = reactive<Set<string>>(
  new Set(JSON.parse(localStorage.getItem("launcher-collapsed-groups") || "[]")),
)
const collapsedEnvironments = reactive<Set<string>>(
  new Set(JSON.parse(localStorage.getItem("launcher-collapsed-environments") || "[]")),
)

const environmentCollapseKey = (groupName: string, environmentKey: string) =>
  `${groupName}::${environmentKey}`

const toggleGroup = (group: string) => {
  if (collapsedGroups.has(group)) {
    collapsedGroups.delete(group)
  } else {
    collapsedGroups.add(group)
  }
  localStorage.setItem("launcher-collapsed-groups", JSON.stringify([...collapsedGroups]))
}

const toggleEnvironment = (groupName: string, environmentKey: string) => {
  const key = environmentCollapseKey(groupName, environmentKey)
  if (collapsedEnvironments.has(key)) {
    collapsedEnvironments.delete(key)
  } else {
    collapsedEnvironments.add(key)
  }
  localStorage.setItem("launcher-collapsed-environments", JSON.stringify([...collapsedEnvironments]))
}

const hasServers = computed(() => servers.value.length > 0)
const hasResults = computed(() => filteredServers.value.length > 0)
const canReorder = computed(() => sortBy.value === "group" && !searchFilter.value.trim().length && !isPersistingOrder.value)

const applyGroupOrder = (groupNames: string[]) => {
  for (const [index, groupName] of groupNames.entries()) {
    for (const server of servers.value) {
      if ((server.group?.trim() || "Ungrouped") === groupName) {
        server.groupOrder = index
      }
    }
  }
}

const applyEnvironmentOrder = (groupName: string, environmentKeys: string[]) => {
  for (const [index, environmentKey] of environmentKeys.entries()) {
    for (const server of servers.value) {
      if ((server.group?.trim() || "Ungrouped") === groupName && ((server.environment?.trim() || "__default__") === environmentKey)) {
        server.environmentOrder = index
      }
    }
  }
}

const applyServerOrder = (groupName: string, environmentKey: string, serverIds: string[]) => {
  for (const [index, serverId] of serverIds.entries()) {
    const server = servers.value.find((item) => item.id === serverId)
    if (server && (server.group?.trim() || "Ungrouped") === groupName && ((server.environment?.trim() || "__default__") === environmentKey)) {
      server.sortOrder = index
    }
  }
}

const persistOrder = async () => {
  isPersistingOrder.value = true
  launchError.value = null
  try {
    for (const server of servers.value) {
      await invoke("save", { ce: JSON.stringify(server) })
    }
  } catch (e) {
    launchError.value = `Saving server order failed: ${e}`
  } finally {
    isPersistingOrder.value = false
  }
}

const moveGroup = (draggedGroupName: string, targetGroupName: string) => {
  if (draggedGroupName === targetGroupName) return

  const reordered = groupedServers.value.map((group) => group.name)
  const draggedIndex = reordered.findIndex((groupName) => groupName === draggedGroupName)
  const targetIndex = reordered.findIndex((groupName) => groupName === targetGroupName)
  if (draggedIndex < 0 || targetIndex < 0) return

  const [moved] = reordered.splice(draggedIndex, 1)
  if (!moved) return
  reordered.splice(targetIndex, 0, moved)
  applyGroupOrder(reordered)
}

const moveEnvironment = (groupName: string, draggedEnvironmentKey: string, targetEnvironmentKey: string) => {
  if (draggedEnvironmentKey === targetEnvironmentKey) return

  const environments = groupedServers.value.find((group) => group.name === groupName)?.environments
  if (!environments) return

  const reordered = [...environments]
  const draggedIndex = reordered.findIndex((environment) => environment.key === draggedEnvironmentKey)
  const targetIndex = reordered.findIndex((environment) => environment.key === targetEnvironmentKey)
  if (draggedIndex < 0 || targetIndex < 0) return

  const [moved] = reordered.splice(draggedIndex, 1)
  if (!moved) return
  reordered.splice(targetIndex, 0, moved)
  applyEnvironmentOrder(groupName, reordered.map((environment) => environment.key))
}

const moveServerWithinBucket = (draggedId: string, targetId: string, groupName: string, environmentKey: string) => {
  if (draggedId === targetId) return

  const bucketServers = groupedServers.value
    .find((group) => group.name === groupName)
    ?.environments.find((environment) => environment.key === environmentKey)
    ?.servers

  if (!bucketServers) return

  const draggedIndex = bucketServers.findIndex((server) => server.id === draggedId)
  const targetIndex = bucketServers.findIndex((server) => server.id === targetId)
  if (draggedIndex < 0 || targetIndex < 0) return

  const reordered = [...bucketServers]
  const [moved] = reordered.splice(draggedIndex, 1)
  if (!moved) return
  reordered.splice(targetIndex, 0, moved)
  applyServerOrder(groupName, environmentKey, reordered.map((server) => server.id))
}

const handleGroupPointerDown = (event: MouseEvent, groupName: string) => {
  if (!canReorder.value || event.button !== 0) return
  event.preventDefault()
  dragging.value = { kind: "group", groupName }
  dropTarget.value = { kind: "group", groupName }
}

const handleEnvironmentPointerDown = (event: MouseEvent, groupName: string, environmentKey: string) => {
  if (!canReorder.value || event.button !== 0) return
  event.preventDefault()
  dragging.value = { kind: "environment", groupName, environmentKey }
  dropTarget.value = { kind: "environment", groupName, environmentKey }
}

const handleServerPointerDown = (event: MouseEvent, serverId: string, groupName: string, environmentKey: string) => {
  if (!canReorder.value || event.button !== 0) return
  event.preventDefault()
  dragging.value = { kind: "server", groupName, environmentKey, serverId }
  dropTarget.value = { kind: "server", groupName, environmentKey, serverId }
}

const handleGroupHover = (targetGroupName: string) => {
  if (!dragging.value || dragging.value.kind !== "group") return
  dropTarget.value = { kind: "group", groupName: targetGroupName }
}

const handleEnvironmentHover = (groupName: string, targetEnvironmentKey: string) => {
  if (!dragging.value || dragging.value.kind !== "environment" || dragging.value.groupName !== groupName) return
  dropTarget.value = { kind: "environment", groupName, environmentKey: targetEnvironmentKey }
}

const handleServerHover = (targetId: string, groupName: string, environmentKey: string) => {
  if (!dragging.value || dragging.value.kind !== "server" || dragging.value.groupName !== groupName || dragging.value.environmentKey !== environmentKey) return
  dropTarget.value = { kind: "server", groupName, environmentKey, serverId: targetId }
}

const finishReorder = async () => {
  if (dragging.value && dropTarget.value) {
    if (dragging.value.kind === "group" && dropTarget.value.kind === "group") {
      moveGroup(dragging.value.groupName, dropTarget.value.groupName)
      await persistOrder()
    } else if (
      dragging.value.kind === "environment"
      && dropTarget.value.kind === "environment"
      && dragging.value.groupName === dropTarget.value.groupName
    ) {
      moveEnvironment(dragging.value.groupName, dragging.value.environmentKey, dropTarget.value.environmentKey)
      await persistOrder()
    } else if (
      dragging.value.kind === "server"
      && dropTarget.value.kind === "server"
      && dragging.value.groupName === dropTarget.value.groupName
      && dragging.value.environmentKey === dropTarget.value.environmentKey
    ) {
      moveServerWithinBucket(dragging.value.serverId, dropTarget.value.serverId, dragging.value.groupName, dragging.value.environmentKey)
      await persistOrder()
    }
  }
  dragging.value = null
  dropTarget.value = null
}

const { trustCertificate } = useConfirmRejectModal()
const handleLaunchClick = (connection: Connection) => {
  isLoading.value = true
  launchError.value = null
  progressMessage.value = "Connecting..."
  nextTick(() => launchServer(connection))
}

const launchServer = async (connection: Connection) => {
  const onProgress = new Channel<{ message: string }>()
  onProgress.onmessage = ({ message }) => {
    progressMessage.value = message
  }

  try {
    // Loop to handle multiple untrusted certs across different jars
    while (true) {
      const response: string = await invoke("launch", {
        id: connection.id,
        on_progress: onProgress,
      })
      const result = JSON.parse(response)

      // Result code 1 means cert needs trust approval
      if (result.code !== 1) return

      const shouldTrustCertificate = await trustCertificate(result.cert)
      if (!shouldTrustCertificate) return

      await invoke("trust_cert", { cert: result.cert.der })
    }
  } catch (e) {
    launchError.value = `Launch failed: ${e}`
  } finally {
    isLoading.value = false
  }
}

const openSettings = (server: Connection) =>
  navigateTo(`/connections/${server.id}`)

const closeImportModal = () => {
  showImportModal.value = false
  isImporting.value = false
  importSourceLabel.value = ""
  importDrafts.value = []
  bulkImportUsername.value = ""
  bulkImportPassword.value = ""
  bulkImportJavaHome.value = ""
}

const closeExportModal = () => {
  showExportModal.value = false
  isExporting.value = false
  exportDrafts.value = []
}

const closeBulkJavaModal = () => {
  showBulkJavaModal.value = false
  isApplyingBulkJava.value = false
  bulkJavaDrafts.value = []
  bulkJavaHomeValue.value = ""
}

const toggleSelectAllImports = () => {
  const nextValue = !allImportSelected.value
  importDrafts.value = importDrafts.value.map((draft) => ({
    ...draft,
    selected: nextValue,
  }))
}

const toggleSelectAllExports = () => {
  const nextValue = !allExportSelected.value
  exportDrafts.value = exportDrafts.value.map((draft) => ({
    ...draft,
    selected: nextValue,
  }))
}

const toggleSelectAllBulkJava = () => {
  const nextValue = !allBulkJavaSelected.value
  bulkJavaDrafts.value = bulkJavaDrafts.value.map((draft) => ({
    ...draft,
    selected: nextValue,
  }))
}

const applyCredentialsToSelectedImports = () => {
  importDrafts.value = importDrafts.value.map((draft) =>
    draft.selected
      ? {
          ...draft,
          username: bulkImportUsername.value,
          password: bulkImportPassword.value,
        }
      : draft,
  )
}

const applyJavaHomeToSelectedImports = (javaHome: string) => {
  importDrafts.value = importDrafts.value.map((draft) =>
    draft.selected
      ? {
          ...draft,
          javaHome,
        }
      : draft,
  )
}

const exportConnections = () => {
  const selectedId = selectedServerId.value
  exportDrafts.value = servers.value
    .map((server) => normalizeExportDraft(server, selectedId ? server.id === selectedId : true))
    .sort(sortByManualOrder)
  showExportModal.value = true
}

const openBulkJavaModal = () => {
  const selectedId = selectedServerId.value
  bulkJavaDrafts.value = servers.value
    .map((server) => normalizeBulkJavaDraft(server, selectedId ? server.id === selectedId : true))
    .sort(sortByManualOrder)
  bulkJavaHomeValue.value = ""
  showBulkJavaModal.value = true
}

const applyBulkJavaHome = (javaHome: string) => {
  bulkJavaDrafts.value = bulkJavaDrafts.value.map((draft) =>
    draft.selected
      ? {
          ...draft,
          javaHome,
        }
      : draft,
  )
}

const confirmExportDrafts = async () => {
  const selectedIds = exportDrafts.value
    .filter((draft) => draft.selected)
    .map((draft) => draft.id)
  if (!selectedIds.length) return

  const filePath = await saveDialog({
    title: "Export Connections",
    defaultPath: selectedIds.length === 1 ? "ballista-server-export.json" : "ballista-connections-export.json",
    filters: [{ name: "JSON", extensions: ["json"] }],
  })
  if (!filePath) return

  try {
    isExporting.value = true
    launchError.value = null
    actionMessage.value = null
    const response: string = await invoke("export_connections", {
      file_path: filePath,
      ids: selectedIds,
    })
    const result = JSON.parse(response)
    const count = result.total ?? selectedIds.length
    closeExportModal()
    actionMessage.value = `Exported ${count} ${count === 1 ? "connection" : "connections"} with embedded portable icons and without credentials.`
  } catch (e) {
    launchError.value = `Export failed: ${e}`
  } finally {
    isExporting.value = false
  }
}

const confirmBulkJavaUpdate = async () => {
  const selectedDrafts = bulkJavaDrafts.value.filter((draft) => draft.selected)
  if (!selectedDrafts.length) return

  try {
    isApplyingBulkJava.value = true
    launchError.value = null
    actionMessage.value = null

    for (const draft of selectedDrafts) {
      await invoke("save", {
        ce: JSON.stringify({
          ...draft,
        }),
      })
    }

    closeBulkJavaModal()
    await loadConnections()
    const javaLabel = selectedDrafts[0]?.javaHome
      ? "the selected Java Home"
      : "System Default Java"
    actionMessage.value = `Updated ${selectedDrafts.length} ${selectedDrafts.length === 1 ? "server" : "servers"} to use ${javaLabel}.`
  } catch (e) {
    launchError.value = `Bulk Java Home update failed: ${e}`
  } finally {
    isApplyingBulkJava.value = false
  }
}

const importConnections = async () => {
  const proceed = await ask(
    "Select a JSON file containing connection definitions. Ballista will strip any saved credentials from the import preview, then let you bulk-apply credentials before saving.",
    { title: "Import Connections", kind: "info" },
  )
  if (!proceed) return
  const filePath = await open({
    title: "Select connections JSON file",
    filters: [{ name: "JSON", extensions: ["json"] }],
    multiple: false,
  })
  if (!filePath) return
  try {
    launchError.value = null
    actionMessage.value = null
    const response: string = await invoke("preview_import", { file_path: filePath })
    const parsed = JSON.parse(response) as Partial<Connection>[]
    importDrafts.value = parsed.map(normalizeImportDraft)
    importSourceLabel.value = typeof filePath === "string" ? filePath.split("/").pop() || filePath : "connections.json"
    bulkImportUsername.value = ""
    bulkImportPassword.value = ""
    bulkImportJavaHome.value = ""
    showImportModal.value = true
  } catch (e) {
    launchError.value = `Import failed: ${e}`
  }
}

const confirmImportDrafts = async () => {
  if (!importDrafts.value.length) return

  try {
    isImporting.value = true
    launchError.value = null
    actionMessage.value = null
    const entries = importDrafts.value.map(({ importKey, selected, ...draft }) => draft)
    let response: string = await invoke("import_entries", {
      entries: JSON.stringify(entries),
      overwrite: false,
    })
    let result = JSON.parse(response)

    if (result.status === "duplicates") {
      const names = result.names.join(", ")
      const confirmed = await ask(
        `${result.names.length} of ${result.total} connections already exist and will be overwritten:\n\n${names}`,
        { title: "Overwrite existing connections?", kind: "warning" },
      )
      if (!confirmed) {
        isImporting.value = false
        return
      }
      response = await invoke("import_entries", {
        entries: JSON.stringify(entries),
        overwrite: true,
      })
      result = JSON.parse(response)
    }

    closeImportModal()
    await loadConnections()
    const importedTotal = result.total ?? entries.length
    actionMessage.value = `Imported ${importedTotal} ${importedTotal === 1 ? "connection" : "connections"}.`
  } catch (e) {
    launchError.value = `Import failed: ${e}`
  } finally {
    isImporting.value = false
  }
}

const refreshStatuses = () => {
  servers.value.forEach(checkConnectivity)
}

const { theme, toggle: toggleTheme } = useTheme()

const showAbout = ref(false)

const openHelp = async () => {
  const confirmed = await ask("This will open the Ballista wiki in your default browser. Continue?", {
    title: "Open Help",
    kind: "info",
  })
  if (confirmed) {
    await shellOpen("https://github.com/pacmano1/launcher/wiki")
  }
}

const deselectAll = () => {
  selectedServerId.value = null
  showSortMenu.value = false
}
</script>

<template>
  <div class="bg-surface-0 flex flex-col h-full select-none overflow-hidden">
    <!-- Header -->
    <div class="flex items-center justify-between px-5 pt-5 pb-3">
      <div class="flex items-center gap-2">
        <button
          class="flex items-center gap-1.5 px-3 py-1.5 text-sm rounded-md bg-accent text-white hover:bg-accent-hover hover:cursor-pointer transition-colors duration-100"
          @click="navigateTo('/connections/new-connection')"
        >
          <icon name="ph:plus-bold" class="text-xs" />
          Add
        </button>
        <button
          class="flex items-center gap-1.5 px-3 py-1.5 text-sm rounded-md border border-border bg-surface-1 text-text-secondary hover:text-text-primary hover:bg-surface-2 hover:cursor-pointer transition-colors duration-100"
          @click="importConnections"
        >
          <icon name="ph:download-simple-bold" class="text-xs" />
          Import
        </button>
        <button
          class="flex items-center gap-1.5 px-3 py-1.5 text-sm rounded-md border border-border bg-surface-1 text-text-secondary hover:text-text-primary hover:bg-surface-2 hover:cursor-pointer transition-colors duration-100"
          @click="exportConnections"
        >
          <icon name="ph:upload-simple-bold" class="text-xs" />
          Export
        </button>
        <button
          class="flex items-center gap-1.5 px-3 py-1.5 text-sm rounded-md border border-border bg-surface-1 text-text-secondary hover:text-text-primary hover:bg-surface-2 hover:cursor-pointer transition-colors duration-100"
          @click="openBulkJavaModal"
        >
          <icon name="ph:coffee-bold" class="text-xs" />
          Bulk Java
        </button>
      </div>
      <div class="flex items-center gap-2">
        <button
          @click="toggleTheme"
          class="flex items-center justify-center size-6 rounded-md text-text-disabled hover:text-text-tertiary hover:cursor-pointer transition-colors duration-100"
        >
          <icon :name="theme === 'dark' ? 'ph:sun' : 'ph:moon'" class="text-sm" />
        </button>
        <button
          @click="openHelp"
          class="flex items-center justify-center size-6 rounded-md text-text-disabled hover:text-text-tertiary hover:cursor-pointer transition-colors duration-100"
        >
          <icon name="ph:question" class="text-sm" />
        </button>
        <button
          @click="showAbout = true"
          class="flex items-center justify-center size-6 rounded-md text-text-disabled hover:text-text-tertiary hover:cursor-pointer transition-colors duration-100"
        >
          <icon name="ph:info" class="text-sm" />
        </button>
      </div>
    </div>

    <!-- Search + Sort -->
    <div class="flex items-center gap-2 px-5 pb-3">
      <div class="relative flex-1">
        <icon
          name="ph:magnifying-glass"
          class="absolute left-2.5 top-1/2 -translate-y-1/2 text-sm text-text-tertiary"
        />
        <input
          type="text"
          placeholder="Search servers..."
          v-model="searchFilter"
          class="w-full bg-surface-1 border border-border rounded-md py-1.5 pl-8 pr-3 text-sm text-text-primary placeholder:text-text-disabled outline-none transition-colors duration-100 focus:border-border-focus focus:ring-1 focus:ring-accent/30"
        />
      </div>
      <button
        @click="refreshStatuses"
        data-tooltip="Refresh server status"
        class="flex items-center justify-center size-8 rounded-md border border-border bg-surface-1 text-text-tertiary hover:text-text-primary hover:cursor-pointer transition-colors duration-100"
      >
        <icon name="ph:arrow-clockwise" class="text-sm" />
      </button>
      <div class="relative">
        <button
          @click="showSortMenu = !showSortMenu"
          class="flex items-center justify-center size-8 rounded-md border border-border bg-surface-1 text-text-tertiary hover:text-text-primary hover:cursor-pointer transition-colors duration-100"
          :class="showSortMenu ? 'border-border-focus text-text-primary' : ''"
        >
          <icon name="ph:sort-ascending" class="text-sm" />
        </button>
        <Transition
          enter-active-class="transition duration-100 ease-out"
          enter-from-class="opacity-0 scale-95"
          enter-to-class="opacity-100 scale-100"
          leave-active-class="transition duration-75 ease-in"
          leave-from-class="opacity-100 scale-100"
          leave-to-class="opacity-0 scale-95"
        >
          <div
            v-if="showSortMenu"
            class="absolute right-0 top-full mt-1 z-50 w-44 bg-surface-1 border border-border rounded-md shadow-lg py-1"
          >
            <button
              v-for="option in ([
                { value: 'group', label: 'Group', icon: 'ph:folders' },
                { value: 'name', label: 'Name', icon: 'ph:sort-ascending' },
                { value: 'lastConnected', label: 'Last connected', icon: 'ph:clock' },
                { value: 'status', label: 'Status', icon: 'ph:circle-half' },
              ] as const)"
              :key="option.value"
              @click="sortBy = option.value"
              class="flex items-center gap-2 w-full px-3 py-1.5 text-xs hover:bg-surface-2 transition-colors duration-75 hover:cursor-pointer"
              :class="sortBy === option.value ? 'text-accent' : 'text-text-secondary'"
            >
              <icon :name="option.icon" class="text-sm" />
              {{ option.label }}
              <icon v-if="sortBy === option.value" name="ph:check-bold" class="text-xs ml-auto" />
            </button>
          </div>
        </Transition>
      </div>
    </div>

    <div
      v-if="sortBy === 'group'"
      class="px-5 pb-2 text-[11px] text-text-disabled"
    >
      {{ searchFilter.trim().length ? "Clear search to drag groups, environments, and servers into a saved order." : "Drag groups, environments, and servers to reorder them." }}
    </div>

    <!-- Server list -->
    <div class="flex-1 overflow-y-auto px-5 pb-5" @click.self="deselectAll">
      <div
        v-if="isInitializing"
        class="flex flex-col items-center justify-center h-full text-center"
      >
        <icon name="ph:circle-notch-bold" class="text-3xl text-accent animate-spin mb-3" />
        <p class="font-medium text-text-secondary">Loading connections...</p>
      </div>

      <div
        v-else-if="initializationError"
        class="flex flex-col items-center justify-center h-full text-center max-w-md mx-auto"
      >
        <icon name="ph:warning-circle" class="text-4xl text-danger mb-3" />
        <p class="font-medium text-text-primary">Ballista couldn't load your connections</p>
        <p class="text-sm text-text-tertiary mt-1 break-words">{{ initializationError }}</p>
        <button
          class="mt-4 flex items-center gap-1.5 px-3 py-1.5 text-sm rounded-md bg-accent text-white hover:bg-accent-hover hover:cursor-pointer transition-colors duration-100"
          @click="loadConnections"
        >
          <icon name="ph:arrow-clockwise-bold" class="text-xs" />
          Retry
        </button>
      </div>

      <!-- No servers empty state -->
      <div
        v-else-if="!hasServers"
        class="flex flex-col items-center justify-center h-full text-center"
      >
        <icon name="ph:hard-drives" class="text-4xl text-text-disabled mb-3" />
        <p class="font-medium text-text-secondary">No servers yet</p>
        <p class="text-sm text-text-tertiary mt-1">Add a connection to get started.</p>
        <button
          class="mt-4 flex items-center gap-1.5 px-3 py-1.5 text-sm rounded-md bg-accent text-white hover:bg-accent-hover hover:cursor-pointer transition-colors duration-100"
          @click="navigateTo('/connections/new-connection')"
        >
          <icon name="ph:plus-bold" class="text-xs" />
          Add Server
        </button>
      </div>

      <!-- No search results empty state -->
      <div
        v-else-if="!hasResults"
        class="flex flex-col items-center justify-center h-full text-center"
      >
        <icon name="ph:magnifying-glass" class="text-4xl text-text-disabled mb-3" />
        <p class="font-medium text-text-secondary">No results</p>
        <p class="text-sm text-text-tertiary mt-1">
          No servers matching "{{ searchFilter }}"
        </p>
      </div>

      <!-- Server groups (grouped mode) -->
      <div v-else-if="isGrouped" class="space-y-4" @click.self="deselectAll">
        <div
          v-for="group in groupedServers"
          :key="group.name"
          @mouseenter="handleGroupHover(group.name)"
          @click.self="deselectAll"
        >
          <button
            class="relative flex items-center gap-1.5 w-full text-xs font-medium text-text-tertiary uppercase tracking-wider px-2 mb-1 rounded-md hover:text-text-secondary transition-colors duration-100 hover:cursor-pointer"
            :class="[
              isGroupTarget(group.name) ? 'before:absolute before:left-2 before:right-2 before:top-0 before:h-0.5 before:rounded-full before:bg-accent' : '',
            ]"
            @click="toggleGroup(group.name)"
          >
            <span
              v-if="canReorder"
              class="flex-none flex items-center justify-center size-5 rounded text-text-disabled cursor-grab active:cursor-grabbing"
              @mousedown.stop="handleGroupPointerDown($event, group.name)"
            >
              <icon name="ph:dots-six-vertical-bold" class="text-sm" />
            </span>
            <icon
              name="ph:caret-right-bold"
              class="text-[10px] transition-transform duration-150"
              :class="collapsedGroups.has(group.name) ? '' : 'rotate-90'"
            />
            {{ group.name }}
            <span class="normal-case tracking-normal font-normal">({{ group.environments.reduce((count, environment) => count + environment.servers.length, 0) }})</span>
          </button>

          <div v-if="!collapsedGroups.has(group.name)" class="space-y-2">
            <div
              v-for="environment in group.environments"
              :key="`${group.name}-${environment.key}`"
              :class="hasNamedEnvironment(environment) ? 'pl-4' : ''"
            >
              <button
                v-if="hasNamedEnvironment(environment)"
                class="relative flex items-center gap-1.5 w-full px-2 mb-1 rounded-md text-[11px] font-medium uppercase tracking-wider text-text-disabled hover:text-text-tertiary transition-colors duration-100 hover:cursor-pointer"
                :class="[
                  isEnvironmentTarget(group.name, environment.key) ? 'before:absolute before:left-2 before:right-2 before:top-0 before:h-0.5 before:rounded-full before:bg-accent' : '',
                ]"
                @mouseenter="handleEnvironmentHover(group.name, environment.key)"
                @click="toggleEnvironment(group.name, environment.key)"
              >
                <span
                  v-if="canReorder"
                  class="flex-none flex items-center justify-center size-5 rounded text-text-disabled cursor-grab active:cursor-grabbing"
                  @mousedown.stop="handleEnvironmentPointerDown($event, group.name, environment.key)"
                >
                  <icon name="ph:dots-six-vertical-bold" class="text-sm" />
                </span>
                <icon
                  name="ph:caret-right-bold"
                  class="text-[9px] transition-transform duration-150"
                  :class="collapsedEnvironments.has(environmentCollapseKey(group.name, environment.key)) ? '' : 'rotate-90'"
                />
                {{ environment.label }}
                <span class="normal-case tracking-normal font-normal">({{ environment.servers.length }})</span>
              </button>
              <div
                v-if="!hasNamedEnvironment(environment) || !collapsedEnvironments.has(environmentCollapseKey(group.name, environment.key))"
                :class="hasNamedEnvironment(environment) ? 'space-y-px pl-3' : 'space-y-px'"
              >
                <div
                  v-for="server in environment.servers"
                  :key="server.id"
                  @mouseenter="handleServerHover(server.id, group.name, environment.key)"
                >
                  <brief-server-info
                    :server="server"
                    :status="serverStatuses[server.id]"
                    :selected="selectedServerId === server.id"
                    :reorder-enabled="canReorder"
                    :reorder-target="isServerTarget(group.name, environment.key, server.id)"
                    @select="selectedServerId = server.id"
                    @launch="handleLaunchClick(server)"
                    @edit="openSettings(server)"
                    @reorder-start="handleServerPointerDown($event, server.id, group.name, environment.key)"
                  />
                </div>
              </div>
            </div>
          </div>
        </div>
      </div>

      <!-- Flat sorted list -->
      <div v-else class="space-y-px" @click.self="deselectAll">
        <brief-server-info
          v-for="server in sortedServers"
          :key="server.id"
          :server="server"
          :status="serverStatuses[server.id]"
          :selected="selectedServerId === server.id"
          @select="selectedServerId = server.id"
          @launch="handleLaunchClick(server)"
          @edit="openSettings(server)"
        />
      </div>
    </div>

    <!-- Bottom status bar -->
    <Transition
      enter-active-class="transition duration-150 ease-out"
      enter-from-class="translate-y-full opacity-0"
      enter-to-class="translate-y-0 opacity-100"
      leave-active-class="transition duration-100 ease-in"
      leave-from-class="translate-y-0 opacity-100"
      leave-to-class="translate-y-full opacity-0"
    >
      <div v-if="isLoading" class="flex-none border-t border-border bg-surface-1">
        <div class="h-0.5 bg-surface-2 overflow-hidden">
          <div class="h-full w-1/3 bg-accent rounded-full animate-[statusSlide_1.5s_ease-in-out_infinite]" />
        </div>
        <div class="flex items-center gap-2 px-4 py-2">
          <icon name="ph:circle-notch-bold" class="text-sm text-accent animate-spin flex-none" />
          <p class="text-xs text-text-secondary truncate">{{ progressMessage }}</p>
        </div>
      </div>
    </Transition>

    <!-- Launch error -->
    <Transition
      enter-active-class="transition duration-150 ease-out"
      enter-from-class="translate-y-full opacity-0"
      enter-to-class="translate-y-0 opacity-100"
      leave-active-class="transition duration-100 ease-in"
      leave-from-class="translate-y-0 opacity-100"
      leave-to-class="translate-y-full opacity-0"
    >
      <div v-if="launchError" class="absolute bottom-0 inset-x-0 bg-danger/10 border-t border-danger/30">
        <div class="flex items-center justify-between px-4 py-2">
          <p class="text-xs text-danger truncate">{{ launchError }}</p>
          <button @click="launchError = null" class="text-xs text-danger hover:text-text-primary hover:cursor-pointer ml-2 flex-none">Dismiss</button>
        </div>
      </div>
    </Transition>

    <Transition
      enter-active-class="transition duration-150 ease-out"
      enter-from-class="translate-y-full opacity-0"
      enter-to-class="translate-y-0 opacity-100"
      leave-active-class="transition duration-100 ease-in"
      leave-from-class="translate-y-0 opacity-100"
      leave-to-class="translate-y-full opacity-0"
    >
      <div v-if="actionMessage" class="absolute bottom-0 inset-x-0 bg-accent/10 border-t border-accent/30">
        <div class="flex items-center justify-between px-4 py-2">
          <p class="text-xs text-text-primary truncate">{{ actionMessage }}</p>
          <button @click="actionMessage = null" class="text-xs text-text-secondary hover:text-text-primary hover:cursor-pointer ml-2 flex-none">Dismiss</button>
        </div>
      </div>
    </Transition>

    <!-- Bulk Java modal -->
    <Transition
      enter-active-class="transition duration-150 ease-out"
      enter-from-class="opacity-0"
      enter-to-class="opacity-100"
      leave-active-class="transition duration-100 ease-in"
      leave-from-class="opacity-100"
      leave-to-class="opacity-0"
    >
      <div v-if="showBulkJavaModal" class="absolute inset-0 z-[100] flex items-center justify-center bg-black/50 px-5" @click.self="closeBulkJavaModal">
        <div class="bg-surface-1 border border-border rounded-lg shadow-overlay w-full max-w-3xl max-h-[85vh] overflow-hidden">
          <div class="flex items-center justify-between px-5 py-4 border-b border-border">
            <div>
              <h2 class="font-semibold text-text-primary">Bulk Update Java Home</h2>
              <p class="text-xs text-text-tertiary mt-1">
                Choose current servers, then apply either a specific Java Home or System Default.
              </p>
            </div>
            <button @click="closeBulkJavaModal" class="text-text-tertiary hover:text-text-primary hover:cursor-pointer">
              <icon name="ph:x" class="text-sm" />
            </button>
          </div>

          <div class="grid md:grid-cols-[280px_minmax(0,1fr)] max-h-[calc(85vh-134px)]">
            <div class="border-b md:border-b-0 md:border-r border-border p-5 space-y-4">
              <div class="flex items-center justify-between">
                <p class="text-sm font-medium text-text-primary">Java Home</p>
                <button
                  class="text-xs text-accent hover:text-accent-hover hover:cursor-pointer"
                  @click="toggleSelectAllBulkJava"
                >
                  {{ allBulkJavaSelected ? "Clear selection" : "Select all" }}
                </button>
              </div>
              <div class="space-y-1">
                <label class="block text-sm font-medium text-text-secondary">Java Home</label>
                <input
                  v-model="bulkJavaHomeValue"
                  type="text"
                  class="w-full bg-surface-0 border border-border rounded-md px-2.5 py-1.5 text-sm text-text-primary outline-none transition-colors duration-100 focus:border-border-focus focus:ring-1 focus:ring-accent/30"
                  placeholder="System default (auto-detected)"
                />
              </div>
              <div class="grid grid-cols-2 gap-2">
                <button
                  class="w-full flex items-center justify-center gap-1.5 px-3 py-2 text-sm rounded-md bg-accent text-white hover:bg-accent-hover hover:cursor-pointer transition-colors duration-100 disabled:opacity-50 disabled:hover:cursor-default"
                  :disabled="selectedBulkJavaCount === 0"
                  @click="applyBulkJavaHome(bulkJavaHomeValue)"
                >
                  <icon name="ph:coffee-bold" class="text-xs" />
                  Apply Path
                </button>
                <button
                  class="w-full flex items-center justify-center gap-1.5 px-3 py-2 text-sm rounded-md border border-border bg-surface-0 text-text-secondary hover:text-text-primary hover:bg-surface-2 hover:cursor-pointer transition-colors duration-100 disabled:opacity-50 disabled:hover:cursor-default"
                  :disabled="selectedBulkJavaCount === 0"
                  @click="applyBulkJavaHome('')"
                >
                  <icon name="ph:desktop-bold" class="text-xs" />
                  System Default
                </button>
              </div>
              <p class="text-xs text-text-tertiary">
                Blank Java Home means Ballista will auto-detect the local JVM at launch time.
              </p>
            </div>

            <div class="overflow-y-auto p-5 space-y-2">
              <div
                v-for="draft in bulkJavaDrafts"
                :key="draft.bulkKey"
                class="rounded-md border px-3 py-2 transition-colors duration-100"
                :class="draft.selected ? 'border-accent/40 bg-accent/5' : 'border-border bg-surface-0'"
              >
                <label class="flex items-start gap-3 hover:cursor-pointer">
                  <input v-model="draft.selected" type="checkbox" class="mt-1 accent-accent" />
                  <div class="min-w-0 flex-1">
                    <div class="flex items-center gap-2">
                      <p class="text-sm font-medium text-text-primary truncate">{{ draft.name }}</p>
                      <span class="text-[10px] uppercase tracking-wide text-text-disabled">
                        {{ draft.group || "Default" }}<template v-if="draft.environment"> / {{ draft.environment }}</template>
                      </span>
                    </div>
                    <p class="text-xs text-text-tertiary truncate mt-0.5">{{ draft.address }}</p>
                    <p class="text-[11px] mt-1" :class="draft.javaHome ? 'text-accent' : 'text-text-disabled'">
                      {{ draft.javaHome ? `Java Home: ${draft.javaHome}` : "Java Home: System default (auto-detected)" }}
                    </p>
                  </div>
                </label>
              </div>
            </div>
          </div>

          <div class="flex items-center justify-between px-5 py-4 border-t border-border">
            <p class="text-xs text-text-tertiary">
              {{ selectedBulkJavaCount }} {{ selectedBulkJavaCount === 1 ? "server" : "servers" }} selected
            </p>
            <div class="flex items-center gap-2">
              <button
                class="px-3 py-1.5 text-sm rounded-md text-text-secondary hover:bg-surface-2 hover:cursor-pointer transition-colors duration-100"
                @click="closeBulkJavaModal"
              >
                Cancel
              </button>
              <button
                class="px-3 py-1.5 text-sm rounded-md bg-accent text-white hover:bg-accent-hover hover:cursor-pointer transition-colors duration-100 disabled:opacity-50 disabled:hover:cursor-default"
                :disabled="isApplyingBulkJava || selectedBulkJavaCount === 0"
                @click="confirmBulkJavaUpdate"
              >
                {{ isApplyingBulkJava ? "Saving..." : `Update ${selectedBulkJavaCount} ${selectedBulkJavaCount === 1 ? "Server" : "Servers"}` }}
              </button>
            </div>
          </div>
        </div>
      </div>
    </Transition>

    <!-- Export modal -->
    <Transition
      enter-active-class="transition duration-150 ease-out"
      enter-from-class="opacity-0"
      enter-to-class="opacity-100"
      leave-active-class="transition duration-100 ease-in"
      leave-from-class="opacity-100"
      leave-to-class="opacity-0"
    >
      <div v-if="showExportModal" class="absolute inset-0 z-[100] flex items-center justify-center bg-black/50 px-5" @click.self="closeExportModal">
        <div class="bg-surface-1 border border-border rounded-lg shadow-overlay w-full max-w-3xl max-h-[85vh] overflow-hidden">
          <div class="flex items-center justify-between px-5 py-4 border-b border-border">
            <div>
              <h2 class="font-semibold text-text-primary">Export Connections</h2>
              <p class="text-xs text-text-tertiary mt-1">
                Choose the servers to include. Credentials are excluded, and custom icons are embedded so the export works on another machine.
              </p>
            </div>
            <button @click="closeExportModal" class="text-text-tertiary hover:text-text-primary hover:cursor-pointer">
              <icon name="ph:x" class="text-sm" />
            </button>
          </div>

          <div class="p-5 border-b border-border flex items-center justify-between gap-3">
            <p class="text-sm text-text-primary">
              {{ selectedExportCount }} of {{ exportDrafts.length }} {{ exportDrafts.length === 1 ? "server" : "servers" }} selected
            </p>
            <button
              class="text-xs text-accent hover:text-accent-hover hover:cursor-pointer"
              @click="toggleSelectAllExports"
            >
              {{ allExportSelected ? "Clear selection" : "Select all" }}
            </button>
          </div>

          <div class="overflow-y-auto max-h-[calc(85vh-174px)] p-5 space-y-2">
            <div
              v-for="draft in exportDrafts"
              :key="draft.exportKey"
              class="rounded-md border px-3 py-2 transition-colors duration-100"
              :class="draft.selected ? 'border-accent/40 bg-accent/5' : 'border-border bg-surface-0'"
            >
              <label class="flex items-start gap-3 hover:cursor-pointer">
                <input v-model="draft.selected" type="checkbox" class="mt-1 accent-accent" />
                <div class="min-w-0 flex-1">
                  <div class="flex items-center gap-2">
                    <p class="text-sm font-medium text-text-primary truncate">{{ draft.name }}</p>
                    <span class="text-[10px] uppercase tracking-wide text-text-disabled">
                      {{ draft.group || "Default" }}<template v-if="draft.environment"> / {{ draft.environment }}</template>
                    </span>
                  </div>
                  <p class="text-xs text-text-tertiary truncate mt-0.5">{{ draft.address }}</p>
                  <p class="text-[11px] mt-1 text-text-disabled">
                    {{ draft.iconDataUrl ? "Includes custom icon" : "No custom icon to embed" }}
                  </p>
                </div>
              </label>
            </div>
          </div>

          <div class="flex items-center justify-between px-5 py-4 border-t border-border">
            <p class="text-xs text-text-tertiary">
              Export format is portable between Ballista installs.
            </p>
            <div class="flex items-center gap-2">
              <button
                class="px-3 py-1.5 text-sm rounded-md text-text-secondary hover:bg-surface-2 hover:cursor-pointer transition-colors duration-100"
                @click="closeExportModal"
              >
                Cancel
              </button>
              <button
                class="px-3 py-1.5 text-sm rounded-md bg-accent text-white hover:bg-accent-hover hover:cursor-pointer transition-colors duration-100 disabled:opacity-50 disabled:hover:cursor-default"
                :disabled="isExporting || selectedExportCount === 0"
                @click="confirmExportDrafts"
              >
                {{ isExporting ? "Exporting..." : `Export ${selectedExportCount} ${selectedExportCount === 1 ? "Server" : "Servers"}` }}
              </button>
            </div>
          </div>
        </div>
      </div>
    </Transition>

    <!-- Import modal -->
    <Transition
      enter-active-class="transition duration-150 ease-out"
      enter-from-class="opacity-0"
      enter-to-class="opacity-100"
      leave-active-class="transition duration-100 ease-in"
      leave-from-class="opacity-100"
      leave-to-class="opacity-0"
    >
      <div v-if="showImportModal" class="absolute inset-0 z-[100] flex items-center justify-center bg-black/50 px-5" @click.self="closeImportModal">
        <div class="bg-surface-1 border border-border rounded-lg shadow-overlay w-full max-w-3xl max-h-[85vh] overflow-hidden">
          <div class="flex items-center justify-between px-5 py-4 border-b border-border">
            <div>
              <h2 class="font-semibold text-text-primary">Review Import</h2>
              <p class="text-xs text-text-tertiary mt-1">
                {{ importSourceLabel }} · credentials are stripped from the file preview until you apply them here.
              </p>
            </div>
            <button @click="closeImportModal" class="text-text-tertiary hover:text-text-primary hover:cursor-pointer">
              <icon name="ph:x" class="text-sm" />
            </button>
          </div>

          <div class="grid md:grid-cols-[280px_minmax(0,1fr)] max-h-[calc(85vh-134px)]">
            <div class="border-b md:border-b-0 md:border-r border-border p-5 space-y-4">
              <div class="flex items-center justify-between">
                <p class="text-sm font-medium text-text-primary">Bulk Credentials</p>
                <button
                  class="text-xs text-accent hover:text-accent-hover hover:cursor-pointer"
                  @click="toggleSelectAllImports"
                >
                  {{ allImportSelected ? "Clear selection" : "Select all" }}
                </button>
              </div>
              <p class="text-xs text-text-tertiary">
                Checked servers will receive the username and password below when you click Apply.
              </p>
              <div class="space-y-1">
                <label class="block text-sm font-medium text-text-secondary">Username</label>
                <input
                  v-model="bulkImportUsername"
                  type="text"
                  class="w-full bg-surface-0 border border-border rounded-md px-2.5 py-1.5 text-sm text-text-primary outline-none transition-colors duration-100 focus:border-border-focus focus:ring-1 focus:ring-accent/30"
                  placeholder="admin"
                />
              </div>
              <div class="space-y-1">
                <label class="block text-sm font-medium text-text-secondary">Password</label>
                <input
                  v-model="bulkImportPassword"
                  type="password"
                  class="w-full bg-surface-0 border border-border rounded-md px-2.5 py-1.5 text-sm text-text-primary outline-none transition-colors duration-100 focus:border-border-focus focus:ring-1 focus:ring-accent/30"
                  placeholder="Password"
                />
              </div>
              <div class="space-y-1">
                <label class="block text-sm font-medium text-text-secondary">Java Home</label>
                <input
                  v-model="bulkImportJavaHome"
                  type="text"
                  class="w-full bg-surface-0 border border-border rounded-md px-2.5 py-1.5 text-sm text-text-primary outline-none transition-colors duration-100 focus:border-border-focus focus:ring-1 focus:ring-accent/30"
                  placeholder="System default (auto-detected)"
                />
              </div>
              <button
                class="w-full flex items-center justify-center gap-1.5 px-3 py-2 text-sm rounded-md bg-accent text-white hover:bg-accent-hover hover:cursor-pointer transition-colors duration-100 disabled:opacity-50 disabled:hover:cursor-default"
                :disabled="selectedImportCount === 0"
                @click="applyCredentialsToSelectedImports"
              >
                <icon name="ph:key-bold" class="text-xs" />
                Apply To {{ selectedImportCount }} {{ selectedImportCount === 1 ? "Server" : "Servers" }}
              </button>
              <div class="grid grid-cols-2 gap-2">
                <button
                  class="w-full flex items-center justify-center gap-1.5 px-3 py-2 text-sm rounded-md border border-border bg-surface-0 text-text-secondary hover:text-text-primary hover:bg-surface-2 hover:cursor-pointer transition-colors duration-100 disabled:opacity-50 disabled:hover:cursor-default"
                  :disabled="selectedImportCount === 0"
                  @click="applyJavaHomeToSelectedImports(bulkImportJavaHome)"
                >
                  <icon name="ph:coffee-bold" class="text-xs" />
                  Apply Java Home
                </button>
                <button
                  class="w-full flex items-center justify-center gap-1.5 px-3 py-2 text-sm rounded-md border border-border bg-surface-0 text-text-secondary hover:text-text-primary hover:bg-surface-2 hover:cursor-pointer transition-colors duration-100 disabled:opacity-50 disabled:hover:cursor-default"
                  :disabled="selectedImportCount === 0"
                  @click="applyJavaHomeToSelectedImports('')"
                >
                  <icon name="ph:desktop-bold" class="text-xs" />
                  System Default
                </button>
              </div>
              <p class="text-xs text-text-tertiary">
                Blank Java Home means Ballista will auto-detect the JVM on this Mac at launch time.
              </p>
            </div>

            <div class="overflow-y-auto p-5 space-y-2">
              <div
                v-for="draft in importDrafts"
                :key="draft.importKey"
                class="rounded-md border px-3 py-2 transition-colors duration-100"
                :class="draft.selected ? 'border-accent/40 bg-accent/5' : 'border-border bg-surface-0'"
              >
                <label class="flex items-start gap-3 hover:cursor-pointer">
                  <input v-model="draft.selected" type="checkbox" class="mt-1 accent-accent" />
                  <div class="min-w-0 flex-1">
                    <div class="flex items-center gap-2">
                      <p class="text-sm font-medium text-text-primary truncate">{{ draft.name }}</p>
                      <span class="text-[10px] uppercase tracking-wide text-text-disabled">
                        {{ draft.group || "Default" }}<template v-if="draft.environment"> / {{ draft.environment }}</template>
                      </span>
                    </div>
                    <p class="text-xs text-text-tertiary truncate mt-0.5">{{ draft.address }}</p>
                    <p class="text-[11px] mt-1" :class="draft.username || draft.password ? 'text-accent' : 'text-text-disabled'">
                      {{ draft.username || draft.password ? `Credentials ready for ${draft.username || "selected user"}` : "No credentials assigned yet" }}
                    </p>
                    <p class="text-[11px] mt-0.5" :class="draft.javaHome ? 'text-accent' : 'text-text-disabled'">
                      {{ draft.javaHome ? `Java Home: ${draft.javaHome}` : "Java Home: System default (auto-detected)" }}
                    </p>
                  </div>
                </label>
              </div>
            </div>
          </div>

          <div class="flex items-center justify-between px-5 py-4 border-t border-border">
            <p class="text-xs text-text-tertiary">
              {{ importDrafts.length }} {{ importDrafts.length === 1 ? "server" : "servers" }} will be imported.
            </p>
            <div class="flex items-center gap-2">
              <button
                class="px-3 py-1.5 text-sm rounded-md text-text-secondary hover:bg-surface-2 hover:cursor-pointer transition-colors duration-100"
                @click="closeImportModal"
              >
                Cancel
              </button>
              <button
                class="px-3 py-1.5 text-sm rounded-md bg-accent text-white hover:bg-accent-hover hover:cursor-pointer transition-colors duration-100 disabled:opacity-50 disabled:hover:cursor-default"
                :disabled="isImporting || importDrafts.length === 0"
                @click="confirmImportDrafts"
              >
                {{ isImporting ? "Importing..." : `Import ${importDrafts.length} ${importDrafts.length === 1 ? "Server" : "Servers"}` }}
              </button>
            </div>
          </div>
        </div>
      </div>
    </Transition>

    <!-- About modal -->
    <Transition
      enter-active-class="transition duration-150 ease-out"
      enter-from-class="opacity-0"
      enter-to-class="opacity-100"
      leave-active-class="transition duration-100 ease-in"
      leave-from-class="opacity-100"
      leave-to-class="opacity-0"
    >
      <div v-if="showAbout" class="absolute inset-0 z-[100] flex items-center justify-center bg-black/50" @click.self="showAbout = false">
        <div class="bg-surface-1 border border-border rounded-lg shadow-overlay w-80 p-5">
          <div class="flex items-center justify-between mb-4">
            <h2 class="font-semibold text-text-primary">About Ballista</h2>
            <button @click="showAbout = false" class="text-text-tertiary hover:text-text-primary hover:cursor-pointer">
              <icon name="ph:x" class="text-sm" />
            </button>
          </div>
          <div class="space-y-3 text-sm">
            <p class="text-text-secondary">Version 2.0.0</p>
            <div class="space-y-1">
              <p class="text-text-secondary">Originally created by <span class="text-text-primary">Kiran Ayyagari</span></p>
              <p class="text-text-secondary">Modifications by <span class="text-text-primary">Diridium Technologies Inc.</span></p>
            </div>
            <p class="text-text-tertiary text-xs">Licensed under MPL-2.0</p>
          </div>
        </div>
      </div>
    </Transition>
  </div>
</template>
