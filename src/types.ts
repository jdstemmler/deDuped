export interface ScanConfig {
  reference_dir: string;
  eval_dir: string;
  dupe_mode: DupeMode;
  move_uniques: boolean;
  unique_dest: string | null;
  categories: string[];
  all_files: boolean;
  hash_algorithm: string;
  custom_extensions: Record<string, string[]>;
  removed_extensions: Record<string, string[]>;
}

export type DupeMode =
  | { type: "Trash" }
  | { type: "MoveToFolder"; dest: string }
  | { type: "ReviewFirst" };

export interface ScanStats {
  ref_collect_ms: number;
  ref_hash_ms: number;
  eval_collect_ms: number;
  eval_hash_ms: number;
  total_ms: number;
  ref_cache_hits: number;
  eval_cache_hits: number;
  ref_file_count: number;
  eval_file_count: number;
  total_bytes: number;
}

export interface ScanResult {
  total_eval: number;
  duplicates: EvalFile[];
  uniques: EvalFile[];
  skipped: number;
  stats: ScanStats;
}

export interface EvalFile {
  path: string;
  relative_path: string;
  size: number;
  hash: string;
  is_duplicate: boolean;
}

export interface ProgressEvent {
  phase: string;
  current: number;
  total: number;
}

export type ActionMode =
  | { type: "Trash" }
  | { type: "MoveToFolder"; dest: string }
  | { type: "Nothing" };

export interface ActionResult {
  processed: number;
  errors: string[];
  dirs_cleaned: number;
}

export interface ActionBatch {
  id: string;
  timestamp: string;
  action_type: string;
  entry_count: number;
  eval_dir: string;
}

export interface FilePreview {
  path: string;
  size: number;
  mime_type: string;
  is_image: boolean;
  thumbnail_data: string | null;
}
