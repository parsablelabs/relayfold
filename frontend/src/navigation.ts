import { useSyncExternalStore } from 'react'

function subscribe(listener: () => void) {
  window.addEventListener('popstate', listener)
  return () => window.removeEventListener('popstate', listener)
}

export function navigate(url: string, replace = false) {
  if (url === window.location.pathname + window.location.search) return
  window.history[replace ? 'replaceState' : 'pushState'](null, '', url)
  window.dispatchEvent(new PopStateEvent('popstate'))
}

export function useLocation(): string {
  return useSyncExternalStore(subscribe, () => window.location.pathname + window.location.search)
}
