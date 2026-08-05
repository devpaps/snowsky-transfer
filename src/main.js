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

  // ── Disk usage
  selectedMountDisk: null,   // DiskUsage | null

  // ── Local files queue
  localFiles: [],         // Array<{ path, name, ext, meta | null }>
  selectedLocal: new Set(),

  // ── Transfer
  transfers:      {},     // id -> { filename, percent, status }
  isTransferring: false,

  // ── Device tab
  activeDeviceTab: 'tracks',
  selectedDevice: new Set(),

  // ── Preview
  previewPath:    null,
  isPlaying:      false,

  // ── Metadata editor modal
  metaModal:      false,
  metaFile:       null,
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
    })
    await listen('transfer:progress', (e) => {
      const { id, percent } = e.payload
      if (this.transfers[id]) this.transfers[id].percent = percent
    })
    await listen('transfer:done', (e) => {
      const { id } = e.payload
      if (this.transfers[id]) this.transfers[id].status = 'done'
    })
    await listen('transfer:error', (e) => {
      const { id, error } = e.payload
      if (this.transfers[id]) {
        this.transfers[id].status = 'error'
        this.transfers[id].error  = error
      }
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

      const file = { path, name, ext, subPath, meta: null, loading: true }
      this.localFiles.push(file)

      invoke('get_local_metadata', { path }).then(meta => {
        file.meta    = meta
        file.loading = false
      }).catch(() => { file.loading = false })
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

  // ─── Transfer ────────────────────────────────────────────────────────────
  async startTransfer() {
    if (!this.selectedLocal.size) return
    if (!this.device && !this.selectedMount) return

    const files = this.selectedLocalFiles

    // Sort by track number so files arrive in order on the device
    const sorted = [...files].sort((a, b) => {
      const ta = a.meta?.track_number ?? 999
      const tb = b.meta?.track_number ?? 999
      return ta - tb
    })

    // If ffmpeg available, ask about conversion (skip MP3 and FLAC)
    const needsConvert = files.some(f => f.ext !== 'mp3' && f.ext !== 'flac')
    if (this.ffmpegAvailable && needsConvert) {
      this.pendingFiles  = sorted
      this.convertModal  = true
      return
    }

    // If using a mount device, copy files directly
    if (this.selectedMount) {
      await this.doMountTransfer(sorted)
      return
    }

    await this.doTransfer(sorted.map(f => this.buildSendRequest(f, null)))
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
    for (const r of requests) {
      if (r.convertTo) {
        try {
          const outPath = await invoke('convert_audio', {
            req: { input_path: r.file.path, output_fmt: r.convertTo, bitrate_kbps: r.bitrate }
          })
          if (this.selectedMount) {
            mountFiles.push({ ...r.file, path: outPath })
          } else {
            sendRequests.push(this.buildSendRequest(r.file, outPath))
          }
        } catch (e) {
          console.error('convert_audio error:', e)
          if (this.selectedMount) {
            mountFiles.push(r.file)
          } else {
            sendRequests.push(this.buildSendRequest(r.file, null))
          }
        }
      } else {
        if (this.selectedMount) {
          mountFiles.push(r.file)
        } else {
          sendRequests.push(this.buildSendRequest(r.file, null))
        }
      }
    }

    if (this.selectedMount) {
      await this.doMountTransfer(mountFiles)
    } else {
      await this.doTransfer(sendRequests)
    }
  },

  buildSendRequest(file, convertedPath) {
    const m = file.meta || {}
    return {
      path:         convertedPath || file.path,
      title:        m.title   || file.name.replace(/\.[^.]+$/, ''),
      artist:       m.artist  || null,
      album:        m.album   || null,
      genre:        m.genre   || null,
      track_number: m.track_number || null,
      duration_ms:  m.duration_ms  || null,
    }
  },

  async doTransfer(requests) {
    this.isTransferring = true
    this.transfers      = {}
    try {
      await invoke('send_tracks', { tracks: requests })
      await this.loadDeviceTracks()
    } catch (e) {
      console.error('send_tracks error:', e)
    } finally {
      this.isTransferring = false
    }
  },

  async doMountTransfer(rawFiles) {
    if (!this.selectedMount) return
    this.isTransferring = true
    this.transfers      = {}
    const dest          = this.selectedMount.mount_path

    // Sort by track number so files arrive in order on the device
    const files = [...rawFiles].sort((a, b) => {
      const ta = a.meta?.track_number ?? 999
      const tb = b.meta?.track_number ?? 999
      return ta - tb
    })

    for (let i = 0; i < files.length; i++) {
      const file   = files[i]
      const id     = `transfer-${i}`

      // Determine target directory and filename
      let targetDir   = dest
      let targetName  = file.name
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

      // Prefix filename with track number for correct alphabetical sort on device
      const trackNum = file.meta?.track_number
      if (trackNum != null) {
        targetName = String(trackNum).padStart(2, '0') + ' - ' + targetName
      }

      this.transfers[id] = { filename: targetName, percent: 0, status: 'transferring', index: i, total: files.length }

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
    }

    // Refresh view after copy
    if (this.viewMode === 'folder') {
      await this.loadFolderContents(dest, this.folderPath)
    } else {
      await this.loadMountTracks(dest)
    }
    this.loadDiskUsage(dest)
    this.isTransferring = false
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
    this.metaForm = file.meta
      ? { ...file.meta }
      : { title: file.name.replace(/\.[^.]+$/, ''), artist: '', album: '', genre: '', year: '', track_number: '' }
    this.metaModal = true
  },

  async saveMeta() {
    if (!this.metaFile) return
    this.metaSaving = true
    try {
      await invoke('update_local_metadata', { path: this.metaFile.path, meta: this.metaForm })
      this.metaFile.meta = { ...this.metaForm }
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
