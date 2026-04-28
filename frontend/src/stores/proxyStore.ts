import { create } from 'zustand'
import type { ProxyStatus } from '@/types'

type ProxyStore = {
  status: ProxyStatus
  set: (s: ProxyStatus) => void
}

export const useProxyStore = create<ProxyStore>((set) => ({
  status: { state: 'stopped' },
  set: (status) => set({ status }),
}))
