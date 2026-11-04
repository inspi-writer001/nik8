import { AppIdentity, createSolanaDevnet, createSolanaTestnet, SolanaCluster } from '@wallet-ui/react-native-kit'

const devnet = createSolanaDevnet({ url: 'https://api.devnet.solana.com' })
const testnet = createSolanaTestnet({ url: 'https://api.testnet.solana.com' })

// EXPO_PUBLIC_NIK8_CLUSTER picks the cluster (devnet | testnet). The Nik8 program uses
// the same ID on both. The gRPC endpoint in .env must serve the same cluster.
const cluster = process.env.EXPO_PUBLIC_NIK8_CLUSTER === 'testnet' ? testnet : devnet

export class AppConfig {
  static identity: AppIdentity = { name: 'communik8' }
  // The first network is the one the app (and the wallet connection) uses.
  static networks: SolanaCluster[] = [cluster, ...[devnet, testnet].filter((network) => network !== cluster)]
  // Demo only: EXPO_PUBLIC_* values are bundled into the app and can be extracted.
  // Use a revocable token.
  static grpc = {
    endpoint: process.env.EXPO_PUBLIC_NIK8_GRPC_ENDPOINT ?? '',
    token: process.env.EXPO_PUBLIC_NIK8_GRPC_TOKEN || null,
  }
  // The `sender` ingress (WebTransport), pinned by certificate hash. Unset = RPC only.
  static ingress = {
    url: process.env.EXPO_PUBLIC_NIK8_INGRESS_URL || null,
    certSha256: process.env.EXPO_PUBLIC_NIK8_INGRESS_CERT_SHA256 || null,
  }
}
