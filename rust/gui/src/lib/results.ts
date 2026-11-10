// Result files of a run: which files a step made, which of them show (the
// Tools page can switch a tool's outputs off), and reading tables.

import type { PlanStep } from './api';

export type Table = { header: string[]; rows: string[][] };

export function parseTsv(text: string, hasHeader = true, sep = '\t'): Table {
  const lines = text.split(/\r?\n/).filter((l) => l.trim());
  const cells = lines.map((l) => l.split(sep));
  return hasHeader ? { header: cells[0] ?? [], rows: cells.slice(1) } : { header: [], rows: cells };
}

/** How a result file is shown: as a table, as text, opened in its own tab, or download only. */
export function fileKind(path: string): 'table' | 'text' | 'page' | 'other' {
  if (/\.(tsv|tab|csv)$/i.test(path) || /summary_(matches|partials|virulence)\.txt$/.test(path) || /\.amrfinder\.out$/.test(path)) return 'table';
  if (/\.(txt|log|json|yml|yaml)$/i.test(path)) return 'text';
  if (/\.html?$/i.test(path)) return 'page';
  return 'other';
}

const ABRICATE = ['#FILE', 'SEQUENCE', 'START', 'END', 'STRAND', 'GENE', 'COVERAGE', 'COVERAGE_MAP', 'GAPS', '%COVERAGE', '%IDENTITY', 'DATABASE', 'ACCESSION', 'PRODUCT', 'RESISTANCE'];

/** Column headings for the files that have no heading line of their own; null when the file has one. */
function headings(path: string, text: string): string[] | null {
  const first = text.split(/\r?\n/).find((l) => l.trim()) ?? '';
  const width = first.split('\t').length;
  if (/\.mlst\.tsv$/.test(path)) return ['Sample', 'Scheme', 'ST', ...Array.from({ length: Math.max(0, width - 3) }, (_, i) => `Allele ${i + 1}`)];
  if (/\.emmtyper\.tsv$/.test(path)) return ['Sample', 'Clusters', 'emm type', 'Other emm-like alleles', 'emm cluster'];
  if (/emmtyper_output\.tsv$/.test(path)) return ['Sample', 'Clusters', 'emm type', 'Other emm-like alleles', 'Isolate'];
  if (/\.abricate\.tsv$/.test(path) && first && !first.startsWith('#FILE')) return ABRICATE;
  return null;
}

/** A file's text as a table, always with column headings (ShigaPass's own files use ';'). */
export function asTable(path: string, text: string): Table {
  const sep = path.endsWith('.csv') ? (text.split('\n')[0].includes(';') ? ';' : ',') : '\t';
  const known = headings(path, text);
  return known ? { header: known, rows: parseTsv(text, false, sep).rows } : parseTsv(text, true, sep);
}

/** The same output of several samples as one table: one heading line, every sample's rows. */
export function combine(parts: { sample: string; path: string; text: string }[]): Table {
  let header: string[] = [];
  const rows: string[][] = [];
  // a "Sample" column in front, unless the rows already start with the sample
  let named: boolean | null = null;
  for (const p of parts) {
    const t = asTable(p.path, p.text);
    if (!t.header.length && !t.rows.length) continue;
    if (named === null) named = p.sample === 'all' || (t.rows[0]?.[0] ?? '').includes(p.sample);
    if (!header.length) header = named ? t.header : ['Sample', ...t.header];
    for (const r of t.rows) rows.push(named ? r : [p.sample, ...r]);
  }
  return { header, rows };
}

/** A tool's output path pattern (`out/{sample}.x.tsv`, `out/*.tsv`, `out/{a,b}*`) as a test on a file name. */
function nameTest(pattern: string): RegExp {
  const name = pattern.split('/').pop() ?? '';
  let re = '';
  for (let i = 0; i < name.length; i++) {
    const c = name[i];
    if (name.startsWith('{sample}', i)) {
      re += '.+';
      i += '{sample}'.length - 1;
    } else if (c === '{') {
      const end = name.indexOf('}', i);
      const inner = name.slice(i + 1, end < 0 ? name.length : end);
      // {1,2} and {a,b}: any one of them
      re += `(?:${inner.split(',').map(escape).join('|')})`;
      i = end < 0 ? name.length : end;
    } else if (c === '*') re += '.*';
    else if (c === '?') re += '.';
    else re += escape(c);
  }
  return new RegExp(`^${re}$`);
}
const escape = (s: string) => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');

/** The folder a step's files are in for one sample ('all' for all-sample steps). */
export function stepFolder(step: PlanStep, sample: string) {
  return (step.publish ?? '').replace('{sample}', sample);
}

/**
 * The result files of a step (for one sample, or of an all-sample step) that
 * show: every file in its folder, minus the outputs switched off on the Tools page.
 */
export function stepFiles(step: PlanStep, sample: string, files: string[], hidden: string[] = []): string[] {
  const folder = stepFolder(step, sample);
  if (!folder) return [];
  const off = hidden.map((o) => step.outputs?.[o]?.path).filter(Boolean).map((p) => nameTest(p!));
  const on = Object.entries(step.outputs ?? {})
    .filter(([o]) => !hidden.includes(o))
    .map(([, v]) => nameTest(v.path));
  return files.filter((f) => {
    if (!f.startsWith(folder + '/')) return false;
    const name = f.slice(folder.length + 1);
    if (name.includes('/')) return false; // a folder inside belongs to another step
    return on.some((t) => t.test(name)) || !off.some((t) => t.test(name));
  });
}

/** One table of a step: the same output of every sample, or one file of an all-sample step. */
export interface View {
  key: string;
  title: string;
  files: { sample: string; path: string }[];
}

/**
 * What a step made, for showing: its tables (per-sample steps: every sample
 * together, one table per output) and its other files (reports, logs ...).
 */
export function stepViews(step: PlanStep, samples: string[], files: string[], hidden: string[] = []): { tables: View[]; others: string[] } {
  const tables: View[] = [];
  const others: string[] = [];
  for (const sample of step.per_sample ? samples : ['all']) {
    for (const path of stepFiles(step, sample, files, hidden)) {
      if (fileKind(path) !== 'table') {
        others.push(path);
        continue;
      }
      const name = path.split('/').pop()!;
      const title = step.per_sample ? name.replace(sample, '').replace(/^[._-]+/, '') : name;
      const key = step.per_sample ? title : path;
      let view = tables.find((v) => v.key === key);
      if (!view) tables.push((view = { key, title, files: [] }));
      view.files.push({ sample, path });
    }
  }
  return { tables, others };
}

/** Friendly titles for the tables. */
export const TABLE_TITLES: Record<string, string> = {
  'mlst_output.tsv': 'MLST',
  'sistr_output.tsv': 'SISTR serovar',
  'emmtyper_output.tsv': 'emm type',
  'abricate_output.tsv': 'ABRicate genes',
  'quast_output.tsv': 'Assembly quality (QUAST)',
  'checkm_depth_output.tsv': 'Completeness, contamination and depth (CheckM)',
  'depth_output.tsv': 'Sequencing depth',
  'bracken_output.tsv': 'Species abundance (Bracken)',
  'staramr_output.tsv': 'AMR and plasmids (staramr)',
  'shigella_typing_report.tsv': 'Shigella typing (ShigaPass and Mykrobe)',
  'shigapass_report.tsv': 'ShigaPass',
  'mykrobe_report.tsv': 'Mykrobe (Shigella)',
  'cdiphtheriae_typing.tsv': 'C. diphtheriae typing',
};

export const titleOf = (path: string) => TABLE_TITLES[path.split('/').pop()!] ?? path.split('/').pop()!;
