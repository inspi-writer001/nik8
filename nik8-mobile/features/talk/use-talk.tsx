import { useCallback, useEffect, useRef, useState } from 'react'
import { PermissionsAndroid } from 'react-native'
import Nik8Native from '@/modules/nik8-native'

const LEVEL_HISTORY = 24
/** Speaker audio within this window counts as "receiving". */
const RECEIVING_WINDOW_MS = 500

async function ensureMicPermission(): Promise<boolean> {
  const permission = PermissionsAndroid.PERMISSIONS.RECORD_AUDIO
  if (await PermissionsAndroid.check(permission)) return true
  const result = await PermissionsAndroid.request(permission, {
    title: 'Microphone',
    message: 'communik8 needs the microphone to send voice while you hold Push.',
    buttonPositive: 'Allow',
  })
  return result === PermissionsAndroid.RESULTS.GRANTED
}

/** Push-to-talk to `recipient`, plus audio levels for the waveform. */
export function useTalk(recipient: string | undefined, enabled: boolean) {
  const [transmitting, setTransmitting] = useState(false)
  const [receiving, setReceiving] = useState(false)
  const [levels, setLevels] = useState<number[]>(() => Array(LEVEL_HISTORY).fill(0))
  const [sentPackets, setSentPackets] = useState(0)
  const [error, setError] = useState<string | null>(null)
  const pressed = useRef(false)
  const lastSpeaker = useRef(0)

  useEffect(() => {
    if (enabled) Nik8Native.prepareToTalk()
  }, [enabled])

  useEffect(() => {
    const subscriptions = [
      Nik8Native.addListener('onTalkSent', ({ packets }) => setSentPackets((count) => count + packets)),
      Nik8Native.addListener('onTalkFailed', ({ reason }) => setError(reason)),
      Nik8Native.addListener('onAudioLevel', ({ source, level }) => {
        if (source === 'speaker' && level > 0) lastSpeaker.current = Date.now()
        setLevels((history) => [...history.slice(1), level])
      }),
    ]
    const timer = setInterval(() => setReceiving(Date.now() - lastSpeaker.current < RECEIVING_WINDOW_MS), 200)
    return () => {
      subscriptions.forEach((subscription) => subscription.remove())
      clearInterval(timer)
    }
  }, [])

  const pressIn = useCallback(async () => {
    if (!enabled || !recipient) return
    pressed.current = true
    setError(null)
    if (!(await ensureMicPermission())) {
      setError('Microphone permission denied')
      return
    }
    // Released while the permission prompt was up.
    if (!pressed.current) return
    try {
      await Nik8Native.startTalking(recipient)
      // A quick tap can release before start resolves; pressOut's stop is queued after it.
      if (pressed.current) setTransmitting(true)
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
    }
  }, [enabled, recipient])

  const pressOut = useCallback(async () => {
    pressed.current = false
    setTransmitting(false)
    await Nik8Native.stopTalking()
  }, [])

  return { transmitting, receiving, levels, sentPackets, error, pressIn, pressOut }
}
