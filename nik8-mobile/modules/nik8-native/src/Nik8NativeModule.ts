import { NativeModule, requireNativeModule } from 'expo'

export interface VoicePacketInfo {
  version: number
  codec: number
  sessionId: string
  recipient: string
  sequence: number
  captureTimestampMs: number
  opusLen: number
}

export type ListenerState =
  | { state: 'connecting' }
  | { state: 'online' }
  | { state: 'reconnecting'; attempt: number; delayMs: number; reason: string }

export interface ListenerStats {
  accepted: number
  duplicates: number
  otherRecipients: number
  rejected: number
  unsupported: number
  lastSequence?: number
}

export interface AudioLevel {
  source: 'mic' | 'speaker'
  /** RMS level in [0, 1]. */
  level: number
}

export interface NearbyPeer {
  /** Session address as hex (32 bytes). */
  address: string
  /** Smoothed signal strength, dBm. */
  rssi: number
  /** Rough estimate from signal strength; absent if the peer didn't send its tx power. */
  distanceMeters?: number
}

type Nik8NativeEvents = {
  onListenerState(event: ListenerState): void
  onListenerStats(event: ListenerStats): void
  onListenerFailed(event: { reason: string }): void
  onTalkSent(event: { packets: number; signature: string }): void
  onTalkFailed(event: { reason: string }): void
  onAudioLevel(event: AudioLevel): void
  onPeers(event: { peers: NearbyPeer[] }): void
  onDiscoveryError(event: { reason: string }): void
}

export interface SessionInfo {
  /** Session key address; the `sender` on every voice packet. */
  pubkey: string
  /** `nik8_user` fee vault PDA. */
  senderVault: string
  balanceLamports: number
  registered: boolean
}

declare class Nik8NativeModule extends NativeModule<Nik8NativeEvents> {
  /**
   * Loads (or creates on first launch) the device session key. Returns its address.
   * With an ingress URL and certificate hash, voice goes through `sender`; RPC otherwise.
   */
  initSession(
    rpcUrl: string,
    grpcEndpoint: string,
    grpcToken: string | null,
    ingressUrl: string | null,
    ingressCertSha256: string | null,
  ): Promise<string>
  /** How voice is submitted, once a session is configured. */
  submitPath(): 'ingress' | 'rpc' | null
  sessionStatus(): Promise<SessionInfo>
  /** Runs `Cre8Account` for the session key. Resolves to the signature, or null if already registered. */
  registerSender(): Promise<string | null>
  /** Sends one silent voice packet to `recipient`. Resolves to the signature. */
  sendTestPacket(recipient: string): Promise<string>
  /** Warms the blockhash cache so the first push sends without delay. */
  prepareToTalk(): void
  /** Starts capturing and sending voice to `recipient`. Needs RECORD_AUDIO. */
  startTalking(recipient: string): Promise<void>
  /** Stops capture and flushes the last audio. */
  stopTalking(): Promise<void>
  /** Advertises `address` (32 bytes) over BLE and scans for other communik8 devices. */
  startDiscovery(address: Uint8Array): void
  stopDiscovery(): void
  version(): string
  parseVoicePacket(data: Uint8Array): VoicePacketInfo
  /** Starts (or restarts) listening for packets addressed to `recipient` (base58). */
  startListening(grpcEndpoint: string, grpcToken: string | null, recipient: string): void
  stopListening(): void
}

export default requireNativeModule<Nik8NativeModule>('Nik8Native')
