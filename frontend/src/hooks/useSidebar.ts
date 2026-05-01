import { create } from 'zustand'
import { persist, createJSONStorage } from 'zustand/middleware'

// Ported from https://github.com/salimi-my/shadcn-ui-sidebar (MIT).
// Differences from the upstream registry:
//   * Plain object spread in `setSettings` instead of immer.produce — keeps
//     us off another runtime dep for a single shallow merge.
//   * `isHoverOpen` defaults to true — that's the whole reason we adopted
//     this sidebar (peek-on-hover is the headline UX).
//   * Persist key `akagi.sidebar` (was `sidebar`) — namespaced like the
//     other Akagi localStorage entries so clearing app state stays scoped.
//   * Dropped the `useStore` hydration shim — Vite is client-only so the
//     persist middleware reads localStorage synchronously on first render.

export type SidebarSettings = { disabled: boolean; isHoverOpen: boolean }

type SidebarStore = {
  isOpen: boolean
  isHover: boolean
  settings: SidebarSettings
  toggleOpen: () => void
  setIsOpen: (isOpen: boolean) => void
  setIsHover: (isHover: boolean) => void
  getOpenState: () => boolean
  setSettings: (settings: Partial<SidebarSettings>) => void
}

export const useSidebar = create(
  persist<SidebarStore>(
    (set, get) => ({
      isOpen: true,
      isHover: false,
      settings: { disabled: false, isHoverOpen: true },
      toggleOpen: () => set({ isOpen: !get().isOpen }),
      setIsOpen: (isOpen) => set({ isOpen }),
      setIsHover: (isHover) => set({ isHover }),
      getOpenState: () => {
        const s = get()
        return s.isOpen || (s.settings.isHoverOpen && s.isHover)
      },
      setSettings: (patch) =>
        set({ settings: { ...get().settings, ...patch } }),
    }),
    {
      name: 'akagi.sidebar',
      storage: createJSONStorage(() => localStorage),
    },
  ),
)
