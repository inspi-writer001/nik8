import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { AccountRole, Address, address, Instruction } from '@solana/kit'
import { useMobileWallet } from '@wallet-ui/react-native-kit'
import Nik8Native from '@/modules/nik8-native'
import { AppConfig } from '@/constants/app-config'
import { useNetwork } from '@/features/network/use-network'

const SYSTEM_PROGRAM = address('11111111111111111111111111111111')
export const LAMPORTS_PER_SOL = 1_000_000_000
/** Covers the sender vault's rent plus a few minutes of packets on testnet. */
export const SESSION_FUNDING_LAMPORTS = 50_000_000n
/** Below this the session key can't pay `Cre8Account` rent and fees. */
export const SESSION_MIN_LAMPORTS = 5_000_000

/** System Program `Transfer` (instruction 2: u32 LE index, u64 LE lamports). */
function transferSol(from: Address, to: Address, lamports: bigint): Instruction {
  const data = new Uint8Array(12)
  const view = new DataView(data.buffer)
  view.setUint32(0, 2, true)
  view.setBigUint64(4, lamports, true)
  return {
    programAddress: SYSTEM_PROGRAM,
    accounts: [
      { address: from, role: AccountRole.WRITABLE_SIGNER },
      { address: to, role: AccountRole.WRITABLE },
    ],
    data,
  }
}

/** The device session key: its balance, registration, and the actions to set it up. */
export function useSession() {
  const { account, sendTransactions } = useMobileWallet()
  const { selectedNetwork } = useNetwork()
  const queryClient = useQueryClient()
  const { endpoint, token } = AppConfig.grpc

  const pubkey = useQuery({
    queryKey: ['session-pubkey', selectedNetwork.url, endpoint, AppConfig.ingress.url],
    queryFn: () =>
      Nik8Native.initSession(selectedNetwork.url, endpoint, token, AppConfig.ingress.url, AppConfig.ingress.certSha256),
    enabled: !!endpoint,
    staleTime: Infinity,
  })

  const status = useQuery({
    queryKey: ['session-status', pubkey.data],
    queryFn: () => Nik8Native.sessionStatus(),
    enabled: !!pubkey.data,
    refetchInterval: 15_000,
  })

  const refresh = () => queryClient.invalidateQueries({ queryKey: ['session-status'] })

  const fund = useMutation({
    mutationFn: async () => {
      if (!account || !pubkey.data) throw new Error('Wallet or session key not ready')
      return sendTransactions([transferSol(account.address, address(pubkey.data), SESSION_FUNDING_LAMPORTS)])
    },
    onSettled: refresh,
  })

  const register = useMutation({
    mutationFn: () => Nik8Native.registerSender(),
    onSettled: refresh,
  })

  const ready = !!status.data?.registered && status.data.balanceLamports >= SESSION_MIN_LAMPORTS

  return { pubkey, status, fund, register, ready }
}
