import { h, ref } from 'vue'
import { getVersion } from '@tauri-apps/api/app'
import { load } from '@tauri-apps/plugin-store'
import { openUrl } from '@tauri-apps/plugin-opener'
import { ElMessage, ElMessageBox } from 'element-plus'
import { i18n } from '../i18n'

export const GITHUB_TOKEN_STORE_KEY = 'github-token'

const REPOSITORY = 'StarBobis/AssetDaemon'
const RELEASES_URL = `https://github.com/${REPOSITORY}/releases`
const API_RELEASES_URL = `https://api.github.com/repos/${REPOSITORY}/releases?per_page=20`

interface GithubRelease {
  tag_name?: string
  name?: string
  body?: string
  html_url?: string
  draft?: boolean
  prerelease?: boolean
  published_at?: string
}

export const isCheckingUpdate = ref(false)

let activeCheck: Promise<void> | null = null

function t(key: string, params?: Record<string, unknown>): string {
  return i18n.global.t(key, params ?? {}) as string
}

function normalizeVersion(value: string): string {
  return value.trim().replace(/^[^\d]*/, '')
}

function compareVersions(left: string, right: string): number {
  const leftParts = normalizeVersion(left).split(/[.-]/).map(part => Number.parseInt(part, 10) || 0)
  const rightParts = normalizeVersion(right).split(/[.-]/).map(part => Number.parseInt(part, 10) || 0)
  const length = Math.max(leftParts.length, rightParts.length)
  for (let index = 0; index < length; index += 1) {
    const diff = (leftParts[index] ?? 0) - (rightParts[index] ?? 0)
    if (diff !== 0) return diff
  }
  return 0
}

function isUsableGithubToken(token: string): boolean {
  const value = token.trim()
  return /^(ghp|gho|ghu|ghs|ghr)_[A-Za-z0-9_]{30,}$/.test(value) ||
    /^github_pat_[A-Za-z0-9_]{50,}$/.test(value)
}

async function loadGithubToken(): Promise<string> {
  try {
    const store = await load('settings.json', { defaults: {}, autoSave: true })
    const token = await store.get<string>(GITHUB_TOKEN_STORE_KEY)
    return typeof token === 'string' && isUsableGithubToken(token) ? token.trim() : ''
  } catch {
    return ''
  }
}

async function fetchLatestRelease(): Promise<GithubRelease | null> {
  const headers: Record<string, string> = {
    Accept: 'application/vnd.github+json',
    'X-GitHub-Api-Version': '2022-11-28',
  }
  const token = await loadGithubToken()
  if (token) {
    headers.Authorization = `Bearer ${token}`
  }

  const response = await fetch(API_RELEASES_URL, { headers })
  if (!response.ok) {
    const body = await response.text().catch(() => '')
    throw new Error(`GitHub API ${response.status}: ${body.slice(0, 240)}`)
  }

  const json = await response.json()
  const releases = Array.isArray(json) ? json as GithubRelease[] : []
  return releases.find(release => !release.draft && !release.prerelease && !!release.tag_name) ?? null
}

async function showUpdateAvailable(currentVersion: string, release: GithubRelease): Promise<void> {
  const latestVersion = release.tag_name || ''
  const releaseNotes = (release.body || '').trim() || t('updateChecker.noReleaseNotes')
  const confirmed = await ElMessageBox.confirm(
    h('div', { class: 'update-checker-message' }, [
      h('p', { style: 'margin: 0 0 10px; line-height: 1.6;' }, t('updateChecker.updateAvailableMessage', {
        current: currentVersion,
        latest: latestVersion,
      })),
      h('div', { style: 'font-weight: 700; margin-bottom: 6px;' }, t('updateChecker.releaseNotes')),
      h('div', {
        style: 'max-height: 300px; overflow: auto; white-space: pre-wrap; line-height: 1.55; padding: 10px; border-radius: 8px; background: var(--el-fill-color-light);',
      }, releaseNotes),
    ]),
    t('updateChecker.updateAvailableTitle'),
    {
      type: 'info',
      confirmButtonText: t('updateChecker.openRelease'),
      cancelButtonText: t('updateChecker.later'),
    },
  ).then(() => true).catch(() => false)

  if (confirmed) {
    await openUrl(release.html_url || RELEASES_URL)
  }
}

export class UpdateChecker {
  static async checkForUpdates(): Promise<void> {
    if (activeCheck) return activeCheck

    activeCheck = (async () => {
      isCheckingUpdate.value = true
      try {
        const [currentVersion, release] = await Promise.all([
          getVersion().catch(() => '0.0.0'),
          fetchLatestRelease(),
        ])
        if (!release?.tag_name) {
          ElMessage.warning(t('updateChecker.noReleaseFound'))
          return
        }

        if (compareVersions(release.tag_name, currentVersion) <= 0) {
          ElMessage.success(t('updateChecker.alreadyLatest', { version: currentVersion }))
          return
        }

        await showUpdateAvailable(currentVersion, release)
      } catch (error) {
        ElMessage.error(t('updateChecker.failed', { error: String(error) }))
      } finally {
        isCheckingUpdate.value = false
        activeCheck = null
      }
    })()

    return activeCheck
  }
}
