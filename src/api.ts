import { invoke } from '@tauri-apps/api/core'

export type Theme = 'system' | 'light' | 'dark'
export type Page = 'repositories' | 'new' | 'open' | 'clone' | 'accounts' | 'settings'
export interface Account {
  id: string
  login: string
}
export interface Repo {
  id: string
  name: string
  path: string
  accountId: string
  fullName: string | null
  firstPush: boolean
}
export interface AutoSyncItem {
  id: string
  status: 'checking' | 'updated' | 'current' | 'skipped' | 'error'
  message: string
  checkedAt: number
}
export interface AppState {
  autoSyncEnabled: boolean
  accounts: Account[]
  activeAccount: string | null
  repos: Repo[]
  theme: Theme
  gitVersion: string | null
  version: string
}
export interface Device {
  session: string
  userCode: string
}
export interface Remote {
  fullName: string
  description: string | null
  private: boolean
}
export interface Info extends Remote {
  name: string
}
export interface Change {
  path: string
  oldPath?: string | null
  index: string
  worktree: string
}
export interface Status {
  branch: string
  unpushed: number
  changes: Change[]
  authorName: string
  authorEmail: string
}
export interface Commit {
  hash: string
  title: string
  author: string
  date: string
}
export interface Branch {
  name: string
  current: boolean
}
export type DeleteMode = 'remove' | 'local' | 'remote' | 'both'
export type Notice = (message: string, error?: boolean) => void
export type FormArgs = Record<string, unknown>

export function api<T = void>(action: string, args: FormArgs = {}): Promise<T> {
  return invoke<T>('dispatch', { action, args })
}

// The backend owns one watcher. Finish detaching the old view before starting the next one.
let lastWatch = Promise.resolve()
export function watchRepository(id: string): Promise<void> {
  const next = lastWatch.then(() => api('repoWatch', { id }))
  lastWatch = next.catch(() => {})
  return next
}

export function errorText(error: unknown) {
  return typeof error === 'string'
    ? error
    : error instanceof Error
      ? error.message
      : 'The operation could not be completed.'
}
export function suggestsGitHubAccess(message: string) {
  return /GitHub App access|installation|GitHub denied permission|repository administration was denied/i.test(message)
}
