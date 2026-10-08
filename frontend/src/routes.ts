export type Page = 'functions' | 'workflows' | 'instances' | 'settings'
export type Route = {
  page: Page
  id?: string
  definitionTab: 'browse' | 'register'
  instanceTab: 'status' | 'events'
}

export function pageUrl(page: Page, id?: string): string {
  return `/${page}${id === undefined ? '' : `/${encodeURIComponent(id)}`}`
}

export function parseRoute(location: string): Route | undefined {
  const url = new URL(location, 'http://localhost')
  const parts = url.pathname.replace(/\/$/, '').split('/').slice(1)
  const page = url.pathname === '/' ? 'instances' : parts[0]
  if (!['functions', 'workflows', 'instances', 'settings'].includes(page) || parts.length > 2 || (page === 'settings' && parts.length > 1)) return undefined
  let id: string | undefined
  try { id = parts[1] === undefined ? undefined : decodeURIComponent(parts[1]) }
  catch { return undefined }
  if (id === '') return undefined
  return {
    page: page as Page,
    id,
    definitionTab: id === undefined && url.searchParams.get('tab') === 'register' ? 'register' : 'browse',
    instanceTab: url.searchParams.get('tab') === 'events' ? 'events' : 'status',
  }
}
