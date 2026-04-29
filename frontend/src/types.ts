// Mirrors backend schema. Source: frontend/README.md.

export type MjaiEvent =
  | { type: 'start_game'; names: string[]; kyoku_first?: number; aka_flag?: boolean; id?: number; num_players?: number }
  | { type: 'start_kyoku'; bakaze: string; dora_marker: string; kyoku: number; honba: number; kyotaku: number; oya: number; scores: number[]; tehais: string[][]; num_players?: number }
  | { type: 'tsumo'; actor: number; pai: string }
  | { type: 'dahai'; actor: number; pai: string; tsumogiri: boolean }
  | { type: 'chi'; actor: number; target: number; pai: string; consumed: [string, string] }
  | { type: 'pon'; actor: number; target: number; pai: string; consumed: [string, string] }
  | { type: 'daiminkan'; actor: number; target: number; pai: string; consumed: [string, string, string] }
  | { type: 'kakan'; actor: number; pai: string; consumed: [string, string, string] }
  | { type: 'ankan'; actor: number; consumed: [string, string, string, string] }
  | { type: 'dora'; dora_marker: string }
  | { type: 'reach'; actor: number }
  | { type: 'reach_accepted'; actor: number }
  | { type: 'hora'; actor: number; target: number; deltas?: number[]; ura_markers?: string[] }
  | { type: 'ryukyoku'; deltas?: number[] }
  | { type: 'kita'; actor: number; pai?: string }
  | { type: 'end_kyoku' }
  | { type: 'end_game' }
  | { type: 'none' }

export type BotResponse = MjaiEvent & { meta?: Record<string, unknown> }

export type BotStatus =
  | { state: 'idle' }
  | { state: 'loading'; bot: string; stage: 'syncing_deps' | 'spawning' }
  | { state: 'ready'; bot: string; actor_id: number }
  | { state: 'error'; bot: string; error: string }
  | { state: 'stopped'; bot: string }

export type ProxyStatus =
  | { state: 'stopped' }
  | { state: 'starting'; addr: string }
  | { state: 'running'; addr: string }
  | { state: 'error'; addr: string | null; error: string }

export type Notification = {
  level: 'info' | 'success' | 'warn' | 'error'
  title: string
  body?: string
  sticky: boolean
  id?: string
}

export type AppConfig = {
  general: { language: string }
  logging: { dir: string; level: string; all_level: string }
  platform: { kind: 'Majsoul' }
  proxy: { enabled: boolean; addr: string; ca_dir: string }
  bot: { enabled: boolean; active_4p: string; active_3p: string; auto_sync: boolean; dir: string }
}

export type FieldKind = 'string' | 'bool' | 'int' | 'float' | 'enum'

export type FieldSpec = {
  type: FieldKind
  label: string
  default: unknown
  help?: string
  secret?: boolean
  min?: number
  max?: number
  step?: number
  choices?: string[]
}

export type Manifest = {
  manifest_version: number
  bot: {
    name: string
    display?: string
    description?: string
    version?: string
    /** Game modes this bot can play. Backend defaults to `["4p"]` when absent. */
    supported_modes: string[]
  }
  source?: { type: 'github_release'; repo: string; asset_glob?: string }
  settings: Record<string, FieldSpec>
}

export type BotInfo = {
  name: string
  dir: string
  has_pyproject: boolean
  manifest?: Manifest
}

export type BotSettings = {
  manifest: Manifest
  values: Record<string, unknown>
}

export type Snapshot = {
  config: AppConfig
  bot_status: BotStatus
  proxy_status: ProxyStatus
  log_dir: string
}

export type WaitInfo = { tile: string; left: number; agari_rate: number | null }

export type ImproveEntry = { draw: string; widened_waits: WaitInfo[]; widened_total: number }

export type Hand13Result = {
  shanten: number
  waits: WaitInfo[]
  waits_total: number
  next_shanten_waits_count: { [tileIdx: number]: number }
  avg_next_shanten_waits: number
  mixed_waits_score: number
  avg_agari_rate: number
  is_furiten: boolean
  furiten_rate: number
  improves: ImproveEntry[]
  improve_way_count: number
  avg_improve_waits_count: number
  dama_point: number
  riichi_point: number
  mixed_round_point: number
  yaku_ids: number[]
}

export type DiscardCandidate = { discard: string; result: Hand13Result }

export type Hand14Result = {
  shanten: number
  maintain: DiscardCandidate[]
  backwards: DiscardCandidate[]
}

export type OpponentRisk = {
  seat: number
  tenpai_rate: number
  risk: number[]
  is_riichi: boolean
}

export type AnalysisResult = {
  seat: number
  turn: number
  shanten: number
  state: 'wait13' | 'discard14'
  hand13: Hand13Result | null
  hand14: Hand14Result | null
  opponents: OpponentRisk[]
  mixed_risk: number[]
  best_attack_discard: string | null
  best_defence_discard: string | null
}

export type DiscardEntry = { tile: string; tedashi: boolean; is_riichi: boolean }

export type MeldSnapshot = {
  kind: 'chi' | 'pon' | 'daiminkan' | 'ankan' | 'kakan'
  tiles: string[]
  from_who: number
  called_tile: string | null
}

export type PlayerSnapshot = {
  seat: number
  tehai: string[]
  melds: MeldSnapshot[]
  river: DiscardEntry[]
  score: number
  riichi_declared: boolean
  riichi_stage: boolean
  double_riichi: boolean
  riichi_declaration_index: number | null
  /** 3p only: north tiles set aside via kita / nukidora. Empty in 4p. */
  kita_tiles: string[]
}

export type GameStateSnapshot = {
  bakaze: 'E' | 'S' | 'W' | 'N'
  kyoku: number
  honba: number
  kyotaku: number
  oya: number
  current_player: number
  turn_count: number
  phase: 'wait_act' | 'wait_response'
  is_done: boolean
  /** 3 (sanma) or 4 (yonma). */
  num_players: number
  /** Length matches num_players. */
  players: PlayerSnapshot[]
  dora_markers: string[]
  our_seat: number | null
}

export type PlayerMahgenView = {
  seat: number
  hand: string
  melds: string[]
  river: string
}

export type MahgenView = {
  /** Length matches num_players. */
  players: PlayerMahgenView[]
  /** 3 (sanma) or 4 (yonma). */
  num_players: number
  dora_indicators: string
}
