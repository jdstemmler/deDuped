export interface ScanConfig {
  reference_dir: string;
  eval_dir: string;
  dupe_mode: DupeMode;
  move_uniques: boolean;
  unique_dest: string | null;
  categories: string[];
  all_files: boolean;
}

export type DupeMode =
  | { type: "Trash" }
  | { type: "MoveToFolder"; dest: string }
  | { type: "ReviewFirst" };

export interface ScanResult {
  total_eval: number;
  duplicates: EvalFile[];
  uniques: EvalFile[];
  skipped: number;
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
