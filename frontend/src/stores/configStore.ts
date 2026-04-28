import { create } from 'zustand'
import type { AppConfig } from '@/types'

type ConfigStore = {
  config: AppConfig | null
  logDir: string
  setConfig: (c: AppConfig) => void
  setLogDir: (p: string) => void
}

export const useConfigStore = create<ConfigStore>((set) => ({
  config: null,
  logDir: '',
  setConfig: (config) => set({ config }),
  setLogDir: (logDir) => set({ logDir }),
}))
