import { useEffect, useState } from 'react'
import type { IconRef } from '@/bindings'
import { readIcon } from '@/lib/icons'
import { iconFor } from './icons'

const cache = new Map<string, string>()

export function ProfileIcon({
  icon,
  className,
}: {
  icon: IconRef | undefined
  className?: string
}) {
  if (icon?.kind === 'custom') {
    return <CustomIcon path={icon.path} className={className} />
  }
  const Icon = iconFor(icon)
  return <Icon className={className} />
}

function CustomIcon({ path, className }: { path: string; className?: string }) {
  const [src, setSrc] = useState<string | null>(() => cache.get(path) ?? null)
  useEffect(() => {
    const cached = cache.get(path)
    if (cached) {
      setSrc(cached)
      return
    }
    let alive = true
    readIcon(path)
      .then((url) => {
        cache.set(path, url)
        if (alive) setSrc(url)
      })
      .catch(() => {})
    return () => {
      alive = false
    }
  }, [path])
  if (!src) return <span className={className} />
  return <img src={src} alt="" role="img" className={className} />
}
