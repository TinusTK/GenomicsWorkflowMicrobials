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
    // inside a folder: one of this step's own outputs (a tool that writes into results/), else another step's
    if (name.includes('/')) return on.some((t) => t.test(name.split('/').pop()!));
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

// ---------------------------------------------------------------- reports as tables
//
// Many tools write a report rather than one table: a text file in sections
// ("Summary", "Lineage report" ...) or a JSON file. These are read into
// tables so every tool's results look alike: a section of `name: value` lines
// becomes one row per sample, a section of rows becomes a table with the
// sample in front.

/** One section of a report. `pairs`: it was `name: value` lines (header = the names, one row). */
export interface Section {
  title: string;
  table: Table;
  pairs: boolean;
}

const isRule = (l: string) => /^\s*[-=~_*]{3,}\s*$/.test(l);
const cell = (v: unknown): string =>
  v === null || v === undefined ? '' : typeof v === 'object' ? (Array.isArray(v) ? v.map(cell).join('; ') : JSON.stringify(v)) : String(v);

/** The lines of one section as a table: `name: value` lines, or rows split on tabs (commas in a .csv). */
function sectionTable(lines: string[], sep: string): { table: Table; pairs: boolean } | null {
  const rows = lines.filter((l) => l.trim());
  if (!rows.length) return null;
  const split = rows.map((l) => l.split(sep).map((c) => c.trim()));
  const width = split[0].length;
  // name: value on every line (a tab or comma between two cells counts too)
  const pairs = rows.map((l, i) => {
    if (split[i].length === 2 && width === 2 && sep !== ':') return split[i];
    const m = l.match(/^\s*([^:\t]{1,60}):\s*(.*)$/);
    return m ? [m[1].trim(), m[2].trim()] : null;
  });
  const colons = rows.filter((l) => /^\s*[^:\t]{1,60}:\s/.test(l) || /^\s*[^:\t]{1,60}:$/.test(l)).length;
  if (pairs.every(Boolean) && (colons === rows.length || (width === 2 && rows.length <= 40 && !looksLikeHeader(split)))) {
    return { table: { header: pairs.map((p) => p![0]), rows: [pairs.map((p) => p![1])] }, pairs: true };
  }
  if (width >= 2 && rows.length >= 2 && split.filter((r) => r.length === width).length >= rows.length * 0.8) {
    return { table: { header: split[0], rows: split.slice(1) }, pairs: false };
  }
  return null;
}

/** Two columns where the first line reads like headings and the rest like values of the same kind. */
function looksLikeHeader(split: string[][]) {
  if (split.length < 3) return false;
  const numeric = (s: string) => s !== '' && !isNaN(Number(s));
  return !numeric(split[0][1]) && split.slice(1).every((r) => numeric(r[1] ?? ''));
}

/** A text report in sections (a title line with a line of ---- or ==== under it), as tables. */
function textSections(text: string, sep: string): Section[] {
  const lines = text.split(/\r?\n/);
  const out: Section[] = [];
  let title = '';
  let body: string[] = [];
  const close = () => {
    const t = sectionTable(body, sep);
    if (t) out.push({ title: title || 'Summary', ...t });
    body = [];
  };
  for (let i = 0; i < lines.length; i++) {
    if (lines[i].trim() && isRule(lines[i + 1] ?? '') && !isRule(lines[i])) {
      close();
      title = lines[i].trim();
      i++;
    } else if (!isRule(lines[i])) body.push(lines[i]);
  }
  close();
  return out;
}

/** A JSON report as tables: its plain values as one row, each list of records as a table. */
function jsonSections(text: string): Section[] {
  let data: unknown;
  try {
    data = JSON.parse(text);
  } catch {
    return [];
  }
  const out: Section[] = [];
  const plain = (v: unknown) => v === null || typeof v !== 'object';
  const visit = (obj: Record<string, unknown>, title: string, depth: number) => {
    const scalars = Object.entries(obj).filter(([, v]) => plain(v));
    if (scalars.length) out.push({ title, pairs: true, table: { header: scalars.map(([k]) => k), rows: [scalars.map(([, v]) => cell(v))] } });
    for (const [k, v] of Object.entries(obj)) {
      if (Array.isArray(v) && v.length && v.every((r) => r && typeof r === 'object' && !Array.isArray(r))) {
        const recs = v as Record<string, unknown>[];
        const header = [...new Set(recs.flatMap((r) => Object.keys(r).filter((c) => plain(r[c]))))];
        if (header.length) out.push({ title: k, pairs: false, table: { header, rows: recs.slice(0, 2000).map((r) => header.map((h) => cell(r[h]))) } });
      } else if (v && typeof v === 'object' && !Array.isArray(v) && depth < 1) {
        visit(v as Record<string, unknown>, k, depth + 1);
      }
    }
  };
  if (Array.isArray(data)) visit({ rows: data }, 'Summary', 1);
  else if (data && typeof data === 'object') visit(data as Record<string, unknown>, 'Summary', 0);
  return out;
}

/** A report file's sections as tables; none when it is not that kind of file. */
export function reportSections(path: string, text: string): Section[] {
  if (/\.json$/i.test(path)) return jsonSections(text);
  if (/\.(txt|csv|tsv|tab|log)$/i.test(path)) return textSections(text, /\.csv$/i.test(path) ? ',' : '\t');
  return [];
}

/**
 * The same report of several samples as one set of tables: a `name: value`
 * section gives one row per sample, a table section every sample's rows with
 * the sample in front.
 */
export function combineReports(parts: { sample: string; path: string; text: string }[]): Section[] {
  const out: Section[] = [];
  for (const p of parts) {
    for (const s of reportSections(p.path, p.text)) {
      let into = out.find((o) => o.title === s.title && o.pairs === s.pairs);
      if (!into) out.push((into = { title: s.title, pairs: s.pairs, table: { header: ['Sample'], rows: [] } }));
      // columns in the order first seen; a sample without one gets a blank
      for (const h of s.table.header) if (!into.table.header.includes(h)) into.table.header.push(h);
      for (const r of s.table.rows) {
        const row = into.table.header.map((h) => (h === 'Sample' ? p.sample : (r[s.table.header.indexOf(h)] ?? '')));
        // a report that names its own sample column keeps it
        if (s.table.header.includes('Sample')) row[0] = r[s.table.header.indexOf('Sample')] ?? p.sample;
        into.table.rows.push(row);
      }
    }
  }
  for (const o of out) for (const r of o.table.rows) while (r.length < o.table.header.length) r.push('');
  return out.filter((o) => o.table.rows.length);
}

/** Kept with a tool's switched-off outputs (Tools page): don't turn its reports into tables. */
export const REPORT_TABLES = '__report_tables';
