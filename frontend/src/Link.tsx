import type { ComponentProps } from 'react'
import { navigate } from './navigation'

export default function Link({ href, onClick, ...props }: ComponentProps<'a'> & { href: string }) {
  return <a {...props} href={href} onClick={event => {
    onClick?.(event)
    if (event.defaultPrevented || event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey || (props.target && props.target !== '_self') || props.download !== undefined) return
    event.preventDefault()
    navigate(href)
  }} />
}
