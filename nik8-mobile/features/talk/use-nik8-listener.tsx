import { useEffect, useState } from 'react'
import Nik8Native, { ListenerState, ListenerStats } from '@/modules/nik8-native'
import { AppConfig } from '@/constants/app-config'
import { useAppActive } from '@/utils/use-app-active'

export type CommsStatus = ListenerState | { state: 'failed'; reason: string } | { state: 'unconfigured' }

/**
 * Listens on Yellowstone for voice packets addressed to `recipient` while mounted and in
 * the foreground. Android blocks network access for background apps, so the listener is
 * stopped on background and restarted fresh (no backoff carried over) on return.
 */
export function useNik8Listener(recipient: string | undefined) {
  const active = useAppActive()
  const [status, setStatus] = useState<CommsStatus>({ state: 'connecting' })
  const [stats, setStats] = useState<ListenerStats | null>(null)
  const [attempt, setAttempt] = useState(0)

  useEffect(() => {
    if (!active || !recipient) {
      return
    }
    const { endpoint, token } = AppConfig.grpc
    if (!endpoint) {
      setStatus({ state: 'unconfigured' })
      return
    }

    setStatus({ state: 'connecting' })
    const subscriptions = [
      Nik8Native.addListener('onListenerState', setStatus),
      Nik8Native.addListener('onListenerStats', setStats),
      Nik8Native.addListener('onListenerFailed', ({ reason }) => setStatus({ state: 'failed', reason })),
    ]

    try {
      Nik8Native.startListening(endpoint, token, recipient)
    } catch (e) {
      setStatus({ state: 'failed', reason: String(e) })
    }

    return () => {
      subscriptions.forEach((subscription) => subscription.remove())
      Nik8Native.stopListening()
    }
  }, [recipient, attempt, active])

  return { status, stats, retry: () => setAttempt((value) => value + 1) }
}
