import Alpine from 'alpinejs'
import { invoke }          from '@tauri-apps/api/core'
import { listen }          from '@tauri-apps/api/event'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { open }            from '@tauri-apps/plugin-dialog'

import './style.css'

// ─── Utility ─────────────────────────────────────────────────────────────────

function fmtDuration(ms) {
  const s = Math.floor(ms / 1000)
  const m = Math.floor(s / 60)
  const h = Math.floor(m / 60)
  const ss = String(s % 60).padStart(2, '0')
  const mm = String(m % 60).padStart(2, '0')
  return h > 0 ? `${h}:${mm}:${ss}` : `${m}:${ss}`
}

function fmtBytes(bytes) {
  if (!Number.isFinite(bytes)) return '0 B'
  if (bytes < 1024)       return `${bytes} B`
  if (bytes < 1048576)    return `${(bytes / 1024).toFixed(1)} KB`
  if (bytes < 1073741824) return `${(bytes / 1048576).toFixed(1)} MB`
  return `${(bytes / 1073741824).toFixed(2)} GB`
}

function extOf(path) {
  return path.split('.').pop().toLowerCase()
}

function trackNumberOf(file) {
  const metadataNumber = Number(file.meta?.track_number)
  if (Number.isFinite(metadataNumber) && metadataNumber > 0) return metadataNumber
  const match = file.name.match(/^\s*(\d{1,3})(?:\s*[-._)]|\s)/)
  return match ? Number(match[1]) : 999
}

function deviceTrackNumber(track) {
  const match = track.filename?.match(/^\s*(\d{1,3})(?:\s*[-._)]|\s)/)
  return match ? Number(match[1]) : (track.track_number || 999)
}

function syncPathKey(path) {
  return path
    .split('/')
    .map(part => part.replace(/^\d{1,3}\s*-\s*/, ''))
    .join('/')
    .toLowerCase()
}

function hasOrderPrefix(name) {
  return /^\s*\d{1,3}(?:\s*[-._)]|\s)/.test(name)
}

const AUDIO_EXTS = new Set(['mp3', 'flac', 'ogg', 'wav', 'm4a', 'aac'])

// ─── Alpine app ──────────────────────────────────────────────────────────────

Alpine.data('app', () => ({

  // ── Device state (MTP)
  device:         null,   // DeviceInfo | null
  isConnecting:   false,
  deviceTracks:   [],
  devicePlaylists: [],
  isLoadingDevice: false,

  // ── Mount device state (mass storage)
  mountDevices:   [],     // MountDevice[] (auto-detected)
  selectedMount:  null,   // MountDevice | null
  isScanningMounts: false,

  // ── Folder view (for mount devices)
  viewMode:       'folder',  // 'folder' | 'flat'
  folderPath:     '',
  folderContents: { directories: [], files: [] },
  folderHistory:  [],        // stack of previous folder paths
  transferDestination: '',
  deviceSearch:   '',
  deviceFormat:   'all',
  isLoadingSearchLibrary: false,

  // ── Disk usage
  selectedMountDisk: null,   // DiskUsage | null

  // ── Local files queue
  localFiles: [],         // Array<{ path, name, ext, size, meta | null }>
  selectedLocal: new Set(),
  draggedLocalPath: null,
  dragOverLocalPath: null,
  pointerDraggingLocal: false,
  pointerDraggingDevice: false,
  draggedDeviceId: null,
  deviceOrderChanged: false,

  // ── Transfer
  transfers:      {},     // id -> { filename, percent, status }
  isTransferring: false,
  cancelTransferRequested: false,
  transferNotice: null,
  transferCloseTimer: null,
  syncSummary: null,
  syncModal: false,

  // ── Device tab
  activeDeviceTab: 'tracks',
  selectedDevice: new Set(),

  // ── Preview
  previewPath:    null,
  isPlaying:      false,

  // ── Metadata editor modal
  metaModal:      false,
  metaFile:       null,
  metaFileType:   'local',
  metaForm:       {},
  metaSaving:     false,

  // ── Conversion modal
  convertModal:   false,
  pendingFiles:   [],     // files awaiting conversion decision
  convertFmt:     'mp3',
  convertBitrate: 192,
  ffmpegAvailable: true,

  // ── Playlist modal
  plModal:        false,
  plName:         '',
  plTracks:       [],

  // ── Drag
  isDragging: false,

  // ─────────────────────────────────────────────────────────────────────────
  async init() {
    // Listen for transfer progress from Rust
    await listen('transfer:start', (e) => {
      const { id, filename, index, total } = e.payload
      this.transfers[id] = { filename, percent: 0, status: 'transferring', index, total }
      this.scrollTransferPanel()
    })
    await listen('transfer:progress', (e) => {
      const { id, percent } = e.payload
      if (this.transfers[id]) this.transfers[id].percent = percent
      this.scrollTransferPanel()
    })
    await listen('transfer:done', (e) => {
      const { id } = e.payload
      if (this.transfers[id]) this.transfers[id].status = 'done'
      this.scrollTransferPanel()
    })
    await listen('transfer:error', (e) => {
      const { id, error } = e.payload
      if (this.transfers[id]) {
        this.transfers[id].status = 'error'
        this.transfers[id].error  = error
      }
      this.scrollTransferPanel()
    })

    // File drag-drop via Tauri window event
    const win = getCurrentWindow()
    await win.onDragDropEvent((event) => {
      if (event.payload.type === 'over') {
        this.isDragging = true
      } else if (event.payload.type === 'leave' || event.payload.type === 'cancelled') {
        this.isDragging = false
      } else if (event.payload.type === 'drop') {
        this.isDragging = false
        const paths = event.payload.paths || []
        this.addFilePaths(paths)
      }
    })

    // Check ffmpeg
    this.ffmpegAvailable = await invoke('ffmpeg_available')
  },

  // ─── Device ──────────────────────────────────────────────────────────────
  async scanDevice() {
    this.isConnecting = true
    try {
      this.device = await invoke('scan_device')
      if (this.device) {
        await this.loadDeviceTracks()
        await this.loadPlaylists()
      } else {
        // No MTP device found — check for mounted mass-storage devices
        await this.detectMounts()
      }
    } catch (e) {
      console.error('scan_device error:', e)
    } finally {
      this.isConnecting = false
    }
  },

  async detectMounts() {
    this.isScanningMounts = true
    try {
      this.mountDevices = await invoke('detect_mounts')
    } catch (e) {
      console.error('detect_mounts error:', e)
    } finally {
      this.isScanningMounts = false
    }
  },

  async selectMountDevice(path) {
    this.isConnecting = true
    try {
      const dev = await invoke('scan_mount_device', { mountPath: path })
      if (dev) {
        this.selectedMount = dev
        this.mountDevices  = []
        this.viewMode      = 'folder'
        this.folderPath    = ''
        this.folderHistory = []
        await this.loadFolderContents(path, '')
        this.loadDiskUsage(path)
      }
    } catch (e) {
      console.error('select_mount_device error:', e)
    } finally {
      this.isConnecting = false
    }
  },

  // ── Folder navigation ──────────────────────────────────────────────

  async loadFolderContents(mountPath, subPath) {
    this.isLoadingDevice = true
    try {
      this.folderContents = await invoke('get_folder_contents', {
        mountPath,
        subPath,
      })
      this.folderPath = subPath
    } catch (e) {
      console.error('get_folder_contents error:', e)
    } finally {
      this.isLoadingDevice = false
    }
  },

  async navigateFolder(subPath) {
    if (!this.selectedMount) return
    this.folderHistory.push(this.folderPath)
    await this.loadFolderContents(this.selectedMount.mount_path, subPath)
  },

  async navigateFolderUp() {
    if (!this.selectedMount || !this.folderContents.parent_rel) return
    this.folderHistory.push(this.folderPath)
    await this.loadFolderContents(
      this.selectedMount.mount_path,
      this.folderContents.parent_rel,
    )
  },

  async navigateFolderBack() {
    const prev = this.folderHistory.pop()
    if (prev === undefined) return
    await this.loadFolderContents(this.selectedMount.mount_path, prev)
  },

  switchViewMode(mode) {
    this.viewMode = mode
    if (mode === 'flat' && this.selectedMount && this.deviceTracks.length === 0) {
      this.loadMountTracks(this.selectedMount.mount_path)
    }
    if (mode === 'folder') {
      this.loadFolderContents(this.selectedMount.mount_path, this.folderPath)
    }
  },

  selectAllFolderFiles() {
    const fileIds = this.folderContents.files.map(f => f.id)
    const allSelected = fileIds.every(id => this.selectedDevice.has(id))
    if (allSelected) {
      for (const id of fileIds) this.selectedDevice.delete(id)
    } else {
      for (const id of fileIds) this.selectedDevice.add(id)
    }
    this.selectedDevice = new Set(this.selectedDevice)
  },

  async deleteFolder(relPath) {
    if (!this.selectedMount) return
    const name = relPath.split('/').pop() || relPath
    if (!window.confirm(`Delete folder "${name}" and all its contents?`)) return
    const fullPath = this.selectedMount.mount_path + '/' + relPath
    try {
      await invoke('delete_mount_folder', { path: fullPath })
      // Remove from selection if any files from that folder were selected
      for (const id of this.selectedDevice) {
        if (id.startsWith(relPath + '/') || id === relPath) {
          this.selectedDevice.delete(id)
        }
      }
      this.selectedDevice = new Set(this.selectedDevice)
      await this.loadFolderContents(this.selectedMount.mount_path, this.folderPath)
      this.loadDiskUsage(this.selectedMount.mount_path)
    } catch (e) {
      console.error('delete_mount_folder error:', e)
    }
  },

  async loadMountTracks(path) {
    this.isLoadingDevice = true
    try {
      this.deviceTracks = await invoke('get_mount_tracks', { mountPath: path })
    } catch (e) {
      console.error('get_mount_tracks error:', e)
    } finally {
      this.isLoadingDevice = false
    }
  },

  async ensureSearchLibraryLoaded() {
    if (!this.selectedMount || this.deviceTracks.length || this.isLoadingSearchLibrary) return
    this.isLoadingSearchLibrary = true
    try {
      await this.loadMountTracks(this.selectedMount.mount_path)
    } finally {
      this.isLoadingSearchLibrary = false
    }
  },

  get isDeviceSearchActive() {
    return this.deviceSearch.trim().length > 0 || this.deviceFormat !== 'all'
  },

  get filteredDeviceTracks() {
    const query = this.deviceSearch.trim().toLowerCase()
    return this.deviceTracks.filter(track => {
      const format = String(track.filetype || track.filename?.split('.').pop() || '').toLowerCase()
      if (this.deviceFormat !== 'all' && format !== this.deviceFormat) return false
      if (!query) return true
      return [track.title, track.artist, track.album, track.filename, track.path, track.id]
        .filter(Boolean)
        .some(value => String(value).toLowerCase().includes(query))
    })
  },

  async loadDiskUsage(path) {
    try {
      this.selectedMountDisk = await invoke('get_disk_usage', { mountPath: path })
    } catch (e) {
      console.error('get_disk_usage error:', e)
    }
  },

  async disconnectMount() {
    this.selectedMount = null
    this.deviceTracks  = []
    this.devicePlaylists = []
    this.selectedDevice.clear()
    this.folderPath    = ''
    this.folderContents = { directories: [], files: [] }
    this.folderHistory  = []
    this.viewMode      = 'folder'
    this.selectedMountDisk = null
  },

  async safeEjectMount() {
    if (!this.selectedMount || this.isTransferring) return
    if (!window.confirm('Sync writes and safely disconnect the device?')) return
    try {
      await invoke('sync_mount', { mountPath: this.selectedMount.mount_path })
      await this.disconnectMount()
      this.transferNotice = 'Device safely disconnected'
    } catch (e) {
      console.error('sync_mount error:', e)
      window.alert(`Could not disconnect the device: ${e}`)
    }
  },

  async disconnectDevice() {
    await invoke('disconnect_device')
    this.device        = null
    this.deviceTracks  = []
    this.devicePlaylists = []
    this.selectedDevice.clear()
    this.mountDevices  = []
    this.selectedMount = null
    this.folderPath    = ''
    this.folderContents = { directories: [], files: [] }
    this.folderHistory  = []
    this.viewMode      = 'folder'
    this.selectedMountDisk = null
  },

  async loadDeviceTracks() {
    this.isLoadingDevice = true
    try {
      this.deviceTracks = await invoke('get_device_tracks')
      this.deviceTracks.sort((a, b) =>
        deviceTrackNumber(a) - deviceTrackNumber(b) ||
        a.filename.localeCompare(b.filename, undefined, { numeric: true })
      )
    } catch (e) {
      console.error('get_device_tracks error:', e)
    } finally {
      this.isLoadingDevice = false
    }
  },

  async loadPlaylists() {
    try {
      this.devicePlaylists = await invoke('get_playlists')
    } catch (e) {
      console.error('get_playlists error:', e)
    }
  },

  async deleteSelectedDeviceTracks() {
    if (!this.selectedDevice.size) return
    const ids = [...this.selectedDevice]
    if (!window.confirm(`Delete ${ids.length} selected ${ids.length === 1 ? 'file' : 'files'}?`)) return

    if (this.selectedMount) {
      // Mount device: delete files by path
      const base = this.selectedMount.mount_path
      for (const id of ids) {
        try {
          const fullPath = base + '/' + id
          await invoke('delete_mount_file', { path: fullPath })
          this.selectedDevice.delete(id)
        } catch (e) {
          console.error('delete_mount_file error:', e)
        }
      }
      // Refresh current view
      if (this.viewMode === 'folder') {
        await this.loadFolderContents(base, this.folderPath)
      } else {
        await this.loadMountTracks(base)
      }
      this.loadDiskUsage(base)
    } else {
      // MTP device: delete by track ID
      for (const id of ids) {
        try {
          await invoke('delete_track', { trackId: id })
          this.deviceTracks = this.deviceTracks.filter(t => t.id !== id)
          this.selectedDevice.delete(id)
        } catch (e) {
          console.error('delete_track error:', e)
        }
      }
    }
  },

  toggleDeviceTrack(id) {
    if (this.selectedDevice.has(id)) this.selectedDevice.delete(id)
    else this.selectedDevice.add(id)
    this.selectedDevice = new Set(this.selectedDevice)
  },

  // ─── Local files ─────────────────────────────────────────────────────────
  async pickFiles() {
    const paths = await open({
      multiple: true,
      filters: [{ name: 'Audio', extensions: ['mp3','flac','ogg','wav','m4a','aac'] }],
    })
    if (paths) this.addFilePaths(Array.isArray(paths) ? paths : [paths])
  },

  async syncFolder() {
    if (!this.selectedMount || this.isTransferring) return
    const folder = await open({ directory: true, multiple: false })
    if (!folder || Array.isArray(folder)) return

    try {
      const sourceFiles = await invoke('expand_audio_path', { path: folder })
      const deviceFiles = await invoke('get_mount_tracks', { mountPath: this.selectedMount.mount_path })
      const rootName = folder.split('/').filter(Boolean).pop() || 'Music'
      const deviceByPath = new Map(deviceFiles.map(file => [syncPathKey(file.id), file]))
      const candidates = []
      let skipped = 0
      let changed = 0

      for (const sourcePath of sourceFiles) {
        const relative = sourcePath.slice(folder.length).replace(/^\/+/, '')
        const targetId = `${rootName}/${relative}`
        const existing = deviceByPath.get(syncPathKey(targetId))
        const sourceSize = Number(await invoke('get_local_file_size', { path: sourcePath }))
        if (existing && Number(existing.file_size) === sourceSize) {
          skipped++
          continue
        }
        if (existing) changed++
        candidates.push({
          path: sourcePath,
          name: sourcePath.split('/').pop(),
          ext: extOf(sourcePath),
          subPath: `${rootName}/${relative}`,
          meta: null,
          loading: true,
        })
      }

      await Promise.all(candidates.map(async file => {
        try { file.meta = await invoke('get_local_metadata', { path: file.path }) } catch (_) { file.meta = null }
        file.loading = false
      }))
      this.syncSummary = { folder: rootName, skipped, changed, pending: candidates.length }

      if (!candidates.length) {
        this.syncModal = true
        return
      }
    if (!window.confirm(`Sync ${candidates.length} new or changed files to the folder "${rootName}"?`)) return

      for (const file of candidates) {
        const relative = file.subPath
        const existing = deviceByPath.get(syncPathKey(relative))
        if (existing) await invoke('delete_mount_file', { path: existing.path })
      }
      this.transferDestination = ''
      await this.doMountTransfer(candidates)
    } catch (e) {
      console.error('syncFolder error:', e)
      window.alert(`Could not sync the folder: ${e}`)
    }
  },

  async addFilePaths(paths) {
    // Resolve each path: single audio file or entire directory tree
    const resolved = [] // { path, subPath }
    for (const p of paths) {
      const files = await invoke('expand_audio_path', { path: p })
      const isDir = files.length !== 1 || files[0] !== p
      for (const f of files) {
        if (!resolved.some(r => r.path === f)) {
          resolved.push({
            path: f,
            subPath: isDir ? p.split('/').pop() + '/' + f.slice(p.length + 1) : null,
          })
        }
      }
    }

    for (const { path, subPath } of resolved) {
      const ext = extOf(path)
      if (!AUDIO_EXTS.has(ext)) continue
      const name = path.split('/').pop()
      if (this.localFiles.some(f => f.path === path)) continue

      const file = { path, name, ext, subPath, meta: null, size: null, loading: true }
      this.localFiles.push(file)

      invoke('get_local_metadata', { path }).then(meta => {
        file.meta    = meta
        file.loading = false
      }).catch(() => { file.loading = false })

      invoke('get_local_file_size', { path }).then(size => {
        file.size = Number(size)
      }).catch(() => { file.size = 0 })
    }
  },

  removeLocalFile(path) {
    this.localFiles     = this.localFiles.filter(f => f.path !== path)
    this.selectedLocal.delete(path)
    this.selectedLocal  = new Set(this.selectedLocal)
  },

  clearLocalFiles() {
    this.localFiles    = []
    this.selectedLocal = new Set()
  },

  toggleLocalFile(path) {
    if (this.selectedLocal.has(path)) this.selectedLocal.delete(path)
    else this.selectedLocal.add(path)
    this.selectedLocal = new Set(this.selectedLocal)
  },

  startLocalDrag(path) {
    this.draggedLocalPath = path
  },

  dragOverLocal(path) {
    if (this.draggedLocalPath && this.draggedLocalPath !== path) this.dragOverLocalPath = path
  },

  dropLocalFile(path) {
    const from = this.localFiles.findIndex(file => file.path === this.draggedLocalPath)
    const to = this.localFiles.findIndex(file => file.path === path)
    if (from < 0 || to < 0 || from === to) return this.cancelLocalDrag()
    const reordered = [...this.localFiles]
    const [file] = reordered.splice(from, 1)
    reordered.splice(to, 0, file)
    this.localFiles = reordered
    this.cancelLocalDrag()
  },

  cancelLocalDrag() {
    this.draggedLocalPath = null
    this.dragOverLocalPath = null
  },

  moveLocalFile(path, direction) {
    const index = this.localFiles.findIndex(file => file.path === path)
    const target = index + direction
    if (index < 0 || target < 0 || target >= this.localFiles.length) return
    const reordered = [...this.localFiles]
    ;[reordered[index], reordered[target]] = [reordered[target], reordered[index]]
    this.localFiles = reordered
  },

  startLocalPointerDrag(path) {
    this.draggedLocalPath = path
    this.pointerDraggingLocal = true
  },

  moveLocalPointer(path) {
    if (!this.pointerDraggingLocal || !this.draggedLocalPath || this.draggedLocalPath === path) return
    const from = this.localFiles.findIndex(file => file.path === this.draggedLocalPath)
    const to = this.localFiles.findIndex(file => file.path === path)
    if (from < 0 || to < 0) return
    const reordered = [...this.localFiles]
    const [file] = reordered.splice(from, 1)
    reordered.splice(to, 0, file)
    this.localFiles = reordered
  },

  endLocalPointerDrag() {
    this.pointerDraggingLocal = false
    this.cancelLocalDrag()
  },

  selectAllLocal() {
    if (this.selectedLocal.size === this.localFiles.length) {
      this.selectedLocal = new Set()
    } else {
      this.selectedLocal = new Set(this.localFiles.map(f => f.path))
    }
  },

  get selectedLocalFiles() {
    return this.localFiles.filter(f => this.selectedLocal.has(f.path))
  },

  get selectedLocalBytes() {
    return this.selectedLocalFiles.reduce((sum, file) => sum + (Number(file.size) || 0), 0)
  },

  get selectedLocalSizesLoading() {
    return this.selectedLocalFiles.some(file => file.size === null || file.size === undefined)
  },

  get selectedLocalOverCapacity() {
    return Boolean(
      this.selectedMountDisk &&
      !this.selectedLocalSizesLoading &&
      this.selectedLocalBytes > Number(this.selectedMountDisk.free_bytes),
    )
  },

  get selectedLocalCapacityUnknown() {
    return Boolean(this.selectedMount && !this.selectedMountDisk)
  },

  async moveDeviceTrack(track, direction) {
    if (!this.selectedMount || this.isTransferring) return
    const files = this.viewMode === 'folder' ? this.folderContents.files : this.deviceTracks
    const index = files.findIndex(file => file.id === track.id)
    const target = index + direction
    if (index < 0 || target < 0 || target >= files.length) return

    const ordered = [...files]
    ;[ordered[index], ordered[target]] = [ordered[target], ordered[index]]
    try {
      await this.renameDeviceOrder(ordered)
    } catch (e) {
      console.error('moveDeviceTrack error:', e)
      window.alert(`Could not change the order: ${e}`)
    }
  },

  startDevicePointerDrag(track) {
    if (!this.selectedMount || this.isTransferring) return
    this.pointerDraggingDevice = true
    this.draggedDeviceId = track.id
    this.deviceOrderChanged = false
  },

  moveDevicePointer(track) {
    if (!this.pointerDraggingDevice || !this.draggedDeviceId || this.draggedDeviceId === track.id) return
    const files = this.viewMode === 'folder' ? this.folderContents.files : this.deviceTracks
    const from = files.findIndex(file => file.id === this.draggedDeviceId)
    const to = files.findIndex(file => file.id === track.id)
    if (from < 0 || to < 0) return
    const reordered = [...files]
    const [file] = reordered.splice(from, 1)
    reordered.splice(to, 0, file)
    if (this.viewMode === 'folder') this.folderContents = { ...this.folderContents, files: reordered }
    else this.deviceTracks = reordered
    this.deviceOrderChanged = true
  },

  async endDevicePointerDrag() {
    if (!this.pointerDraggingDevice) return
    this.pointerDraggingDevice = false
    this.draggedDeviceId = null
    if (!this.deviceOrderChanged) return
    this.deviceOrderChanged = false
    const files = this.viewMode === 'folder' ? this.folderContents.files : this.deviceTracks
    try {
      await this.renameDeviceOrder(files)
    } catch (e) {
      console.error('device reorder error:', e)
      window.alert(`Could not change the order: ${e}`)
      if (this.selectedMount) {
        if (this.viewMode === 'folder') await this.loadFolderContents(this.selectedMount.mount_path, this.folderPath)
        else await this.loadMountTracks(this.selectedMount.mount_path)
      }
    }
  },

  async renameDeviceOrder(ordered) {
    const files = ordered.map((file, index) => {
      const name = file.id.split('/').pop()
      const cleanName = name.replace(/^\d{1,3}\s*-\s*/, '')
      const parent = file.id.includes('/') ? file.id.slice(0, file.id.lastIndexOf('/') + 1) : ''
      return { oldPath: file.id, newPath: `${parent}${String(index + 1).padStart(2, '0')} - ${cleanName}` }
    })
    await invoke('rename_mount_files', {
      mountPath: this.selectedMount.mount_path,
      files,
    })
    if (this.viewMode === 'folder') {
      await this.loadFolderContents(this.selectedMount.mount_path, this.folderPath)
    } else {
      await this.loadMountTracks(this.selectedMount.mount_path)
    }
  },

  // ─── Transfer ────────────────────────────────────────────────────────────
  async startTransfer() {
    if (!this.selectedLocal.size) return
    if (!this.device && !this.selectedMount) return

    const files = this.selectedLocalFiles
    if (files.some(file => file.loading)) {
      window.alert('Wait until the track metadata has loaded before transferring.')
      return
    }

    if (this.selectedMount) {
      if (this.selectedLocalSizesLoading) {
        window.alert('Wait until the selected file sizes have loaded before transferring.')
        return
      }
      if (this.selectedLocalCapacityUnknown) {
        window.alert('Wait until the device storage information has loaded before transferring.')
        return
      }
      if (this.selectedLocalOverCapacity) {
        const excess = this.selectedLocalBytes - Number(this.selectedMountDisk.free_bytes)
        window.alert(`Not enough space. You need ${fmtBytes(excess)} less space.`)
        return
      }
    }

    // Preserve the order chosen in the local file queue.
    const ordered = [...files]

    // If ffmpeg available, ask about conversion (skip MP3 and FLAC)
    const needsConvert = files.some(f => f.ext !== 'mp3' && f.ext !== 'flac')
    if (this.ffmpegAvailable && needsConvert) {
      this.pendingFiles  = ordered
      this.convertModal  = true
      return
    }

    // If using a mount device, copy files directly
    if (this.selectedMount) {
      await this.doMountTransfer(ordered)
      return
    }

    await this.doTransfer(ordered.map((f, i) => this.buildSendRequest(f, null, i + 1)))
  },

  async confirmConvert(convert) {
    this.convertModal = false
    const requests = this.pendingFiles.map(f => {
      if (convert && f.ext !== 'mp3') {
        return { file: f, convertTo: this.convertFmt, bitrate: this.convertBitrate }
      }
      return { file: f, convertTo: null }
    })
    this.pendingFiles = []

    // Convert files that need it first
    const sendRequests = []
    const mountFiles   = []
    for (const [index, r] of requests.entries()) {
      if (r.convertTo) {
        try {
          const outPath = await invoke('convert_audio', {
            req: { input_path: r.file.path, output_fmt: r.convertTo, bitrate_kbps: r.bitrate }
          })
          if (this.selectedMount) {
              mountFiles.push({ ...r.file, originalPath: r.file.path, path: outPath })
          } else {
            sendRequests.push(this.buildSendRequest(r.file, outPath, index + 1))
          }
        } catch (e) {
          console.error('convert_audio error:', e)
          if (this.selectedMount) {
            mountFiles.push(r.file)
          } else {
            sendRequests.push(this.buildSendRequest(r.file, null, index + 1))
          }
        }
      } else {
        if (this.selectedMount) {
          mountFiles.push(r.file)
        } else {
          sendRequests.push(this.buildSendRequest(r.file, null, index + 1))
        }
      }
    }

    if (this.selectedMount) {
      await this.doMountTransfer(mountFiles)
    } else {
      await this.doTransfer(sendRequests)
    }
  },

  buildSendRequest(file, convertedPath, orderNumber = 1) {
    const m = file.meta || {}
    const sourceName = file.name.replace(/\.[^.]+$/, '')
    const extension = (convertedPath || file.path).split('.').pop()
    const filename = hasOrderPrefix(sourceName)
      ? `${sourceName}.${extension}`
      : `${String(orderNumber).padStart(2, '0')} - ${sourceName}.${extension}`
    return {
      path:         convertedPath || file.path,
      filename,
      title:        m.title   || file.name.replace(/\.[^.]+$/, ''),
      artist:       m.artist  || null,
      album:        m.album   || null,
      genre:        m.genre   || null,
      track_number: orderNumber,
      duration_ms:  m.duration_ms  || null,
    }
  },

  async doTransfer(requests) {
    this.isTransferring = true
    this.transfers      = {}
    this.transferNotice = null
    clearTimeout(this.transferCloseTimer)
    try {
      await invoke('send_tracks', { tracks: requests })
      await this.loadDeviceTracks()
    } catch (e) {
      console.error('send_tracks error:', e)
      this.transferNotice = `Could not start transfer: ${e}`
    } finally {
      this.isTransferring = false
      this.finishTransfer()
    }
  },

  async doMountTransfer(rawFiles) {
    if (!this.selectedMount) return
    this.isTransferring = true
    this.cancelTransferRequested = false
    this.transfers      = {}
    this.transferNotice = null
    clearTimeout(this.transferCloseTimer)
    const destination   = this.transferDestination || (this.viewMode === 'folder' ? this.folderPath : '')
    const dest          = this.selectedMount.mount_path + (destination ? '/' + destination : '')

    // Preserve the order chosen in the local file queue.
    const files = [...rawFiles]

    for (let i = 0; i < files.length; i++) {
      if (this.cancelTransferRequested) break
      const file   = files[i]
      const id     = `transfer-${i}`

      // Determine target directory and filename
      let targetDir   = dest
      let targetName  = file.name
      if (file.originalPath && file.path !== file.originalPath) {
        targetName = file.name.replace(/\.[^.]+$/, '.' + file.path.split('.').pop())
      }
      if (file.subPath) {
        const parts = file.subPath.split('/')
        targetName = parts.pop()
        const subDir = parts.join('/')
        if (subDir) {
          targetDir = dest + '/' + subDir
          try {
            await invoke('create_dir_all', { path: targetDir })
          } catch (e) {
            console.error('create_dir_all error:', e)
            this.transfers[id] = { filename: targetName, percent: 0, status: 'error', error: e.toString(), index: i, total: files.length }
            continue
          }
        }
      }

      // Preserve an existing order prefix instead of duplicating it.
      if (!hasOrderPrefix(targetName)) {
        targetName = String(i + 1).padStart(2, '0') + ' - ' + targetName
      }

      this.transfers[id] = { filename: targetName, percent: 0, status: 'transferring', index: i, total: files.length, sourceFile: file }
      this.scrollTransferPanel()

      try {
        await invoke('copy_to_device', {
          source:      file.path,
          destDir:     targetDir,
          filename:    targetName,
          transferId:  id,
        })
        this.transfers[id].status = 'done'
        this.transfers[id].percent = 100
      } catch (e) {
        console.error('copy_to_device error:', e)
        this.transfers[id].status = 'error'
        this.transfers[id].error  = e.toString()
      }
      if (file.originalPath && file.path !== file.originalPath) {
        await invoke('cleanup_temp_file', { path: file.path }).catch(error => console.error('cleanup_temp_file error:', error))
      }
    }

    // Refresh view after copy
    if (this.viewMode === 'folder') {
      await this.loadFolderContents(this.selectedMount.mount_path, destination)
    } else {
      await this.loadMountTracks(this.selectedMount.mount_path)
    }
    this.loadDiskUsage(this.selectedMount.mount_path)
    this.isTransferring = false
    this.finishTransfer()
    this.cancelTransferRequested = false
  },

  scrollTransferPanel() {
    this.$nextTick(() => {
      const panel = this.$refs.transferPanel
      if (panel) panel.scrollTop = panel.scrollHeight
    })
  },

  finishTransfer() {
    const transfers = this.transferList
    const done = transfers.filter(t => t.status === 'done').length
    const failed = transfers.filter(t => t.status === 'error')
    const errors = failed.length
    if (!transfers.length) return
    const cancelled = this.cancelTransferRequested

    this.transferNotice = cancelled
      ? `Transfer cancelled (${done} completed)`
      : errors
      ? `Transfer failed for ${failed.map(t => `${t.filename}: ${t.error || 'unknown error'}`).join(' | ')}`
      : `${done} ${done === 1 ? 'file' : 'files'} transferred successfully`

    this.transferCloseTimer = setTimeout(() => {
      if (!this.isTransferring) this.transfers = {}
    }, 3500)
  },

  cancelTransfer() {
    this.cancelTransferRequested = true
  },

  async retryTransfer(id) {
    const transfer = this.transfers[id]
    if (!transfer?.sourceFile || transfer.sourceFile.originalPath || this.isTransferring) return
    await this.doMountTransfer([transfer.sourceFile])
  },

  setTransferDestination() {
    this.transferDestination = this.folderPath
  },

  get transferList() {
    return Object.entries(this.transfers).map(([id, t]) => ({ id, ...t }))
  },

  get allTransfersDone() {
    return this.transferList.length > 0 &&
      this.transferList.every(t => t.status === 'done' || t.status === 'error')
  },

  // ─── Preview ─────────────────────────────────────────────────────────────
  async togglePreview(path) {
    if (this.previewPath === path && this.isPlaying) {
      await invoke('stop_preview')
      this.isPlaying   = false
      this.previewPath = null
    } else {
      await invoke('preview_track', { path })
      this.previewPath = path
      this.isPlaying   = true
    }
  },

  // ─── Metadata editor ─────────────────────────────────────────────────────
  async openMetaEditor(file) {
    this.metaFile = file
    this.metaFileType = 'local'
    this.metaForm = file.meta
      ? { ...file.meta }
      : { title: file.name.replace(/\.[^.]+$/, ''), artist: '', album: '', genre: '', year: '', track_number: '' }
    this.metaModal = true
  },

  openDeviceMetaEditor(track) {
    this.metaFile = track
    this.metaFileType = 'device'
    this.metaForm = {
      title: track.title || '',
      artist: track.artist || '',
      album: track.album || '',
      genre: track.genre || '',
      year: track.year || '',
      track_number: track.track_number || '',
      duration_ms: track.duration_ms || 0,
      cover_art: track.cover_art || null,
    }
    this.metaModal = true
  },

  async saveMeta() {
    if (!this.metaFile) return
    this.metaSaving = true
    try {
      await invoke('update_local_metadata', { path: this.metaFile.path, meta: this.metaForm })
      if (this.metaFileType === 'device') {
        Object.assign(this.metaFile, {
          title: this.metaForm.title || null,
          artist: this.metaForm.artist || null,
          album: this.metaForm.album || null,
          genre: this.metaForm.genre || null,
          year: this.metaForm.year || null,
          track_number: this.metaForm.track_number || null,
          cover_art: this.metaForm.cover_art || null,
        })
        if (this.selectedMount) {
          if (this.viewMode === 'folder') {
            await this.loadFolderContents(this.selectedMount.mount_path, this.folderPath)
          } else {
            await this.loadMountTracks(this.selectedMount.mount_path)
          }
        }
      } else {
        this.metaFile.meta = { ...this.metaForm }
      }
      this.metaModal     = false
    } catch (e) {
      console.error('update_local_metadata error:', e)
    } finally {
      this.metaSaving = false
    }
  },

  // ─── Playlists ───────────────────────────────────────────────────────────
  openNewPlaylist() {
    this.plName   = ''
    this.plTracks = [...this.selectedDevice]
    this.plModal  = true
  },

  async savePlaylist() {
    if (!this.plName.trim()) return
    try {
      await invoke('create_playlist', { name: this.plName, trackIds: this.plTracks })
      await this.loadPlaylists()
      this.plModal = false
    } catch (e) {
      console.error('create_playlist error:', e)
    }
  },

  async deletePlaylist(id) {
    if (!window.confirm('Delete this playlist? Files on the device will not be affected.')) return
    try {
      await invoke('delete_playlist', { playlistId: id })
      await this.loadPlaylists()
    } catch (e) {
      console.error('delete_playlist error:', e)
    }
  },

  // ─── Helpers ─────────────────────────────────────────────────────────────
  fmtDuration,
  fmtBytes,
}))

Alpine.start()
