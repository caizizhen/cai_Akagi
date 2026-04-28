import { create } from 'zustand'
import type { GameStateSnapshot, MahgenView } from '@/types'

type GameStore = {
  game: GameStateSnapshot | null
  view: MahgenView | null
  setGame: (g: GameStateSnapshot | null) => void
  setView: (v: MahgenView | null) => void
}

export const useGameStore = create<GameStore>((set) => ({
  game: null,
  view: null,
  setGame: (game) => set({ game }),
  setView: (view) => set({ view }),
}))
