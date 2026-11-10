// Reads the summary tables a run makes into one row per sample for the
// "At a glance" table: organism, MLST, serovar / emm, AMR genes, quality.

import { fileText } from './api';

export type Table = { header: string[]; rows: string[][] };

export function parseTsv(text: string, hasHeader = true): Table {
  const lines = text.split(/\r?\n/).filter((l) => l.trim());
  const cells = lines.map((l) => l.split('\t'));
  return hasHeader ? { header: cells[0] ?? [], rows: cells.slice(1) } : { header: [], rows: cells };
}

/** "S0000066_CTRL_SAL_19102026_S29_L001.fasta" -> "S0000066_CTRL_SAL_19102026_S29_L001" */
export function sampleOf(name: string) {
  return name
    .split(/[\\/]/)
    .pop()!
    .replace(/\.(contigs\.fa|fasta|fa|fna|tmp|tsv|tab)$/i, '');
}

export interface Glance {
  sample: string;
  scheme?: string;
  st?: string;
  serovar?: string;
  emm?: string;
  amr: string[];
  completeness?: string;
  contamination?: string;
  depth?: string;
  species?: string;
}

const col = (t: Table, name: string) => t.header.findIndex((h) => h.trim().toLowerCase() === name.toLowerCase());

export async function glance(runId: string, samples: string[], files: string[]): Promise<Glance[]> {
  const rows = new Map<string, Glance>(samples.map((s) => [s, { sample: s, amr: [] }]));
  const row = (name: string) => {
    const s = sampleOf(name);
    if (!rows.has(s)) rows.set(s, { sample: s, amr: [] });
    return rows.get(s)!;
  };
  const read = async (path: string, header = true) => {
    if (!files.includes(path)) return null;
    const text = await fileText(runId, path);
    return text ? parseTsv(text, header) : null;
  };

  const [mlst, sistr, emm, checkm, abricate, bracken] = await Promise.all([
    read('summary/mlst_output.tsv'),
    read('summary/sistr_output.tsv'),
    read('summary/emmtyper_output.tsv', false),
    read('summary/checkm_depth_output.tsv'),
    read('summary/abricate_output.tsv'),
    read('summary/bracken_output.tsv'),
  ]);
  if (mlst) {
    const [n, sc, st] = [col(mlst, 'name'), col(mlst, 'scheme'), col(mlst, 'ST')];
    for (const r of mlst.rows) Object.assign(row(r[n] ?? r[0]), { scheme: r[sc], st: r[st] && r[st] !== '-' ? `ST${r[st]}` : 'no ST' });
  }
  if (sistr) {
    const [n, sv] = [col(sistr, 'name'), col(sistr, 'serovar')];
    // SISTR writes "-:-:-" for a non-Salmonella genome: no serovar
    for (const r of sistr.rows) if (r[sv] && !/^[-:\s]*$/.test(r[sv])) row(r[n] ?? r[0]).serovar = r[sv];
  }
  if (emm) {
    for (const r of emm.rows) if (r[2] && r[2] !== '-') row(r[0]).emm = r[2];
  }
  if (checkm) {
    const [n, c, x] = [col(checkm, 'Bin Id'), col(checkm, 'Completeness'), col(checkm, 'Contamination')];
    for (const r of checkm.rows) {
      const g = row(r[n] ?? r[0]);
      g.completeness = r[c];
      g.contamination = r[x];
      g.depth = r[r.length - 1]?.trim();
    }
  }
  if (abricate) {
    const [f, gene, db] = [col(abricate, '#FILE'), col(abricate, 'GENE'), col(abricate, 'DATABASE')];
    for (const r of abricate.rows) {
      if (r[f] === '#FILE' || (db >= 0 && r[db] !== 'ncbi')) continue;
      const g = row(r[f]);
      if (r[gene] && !g.amr.includes(r[gene])) g.amr.push(r[gene]);
    }
  }
  if (bracken) {
    // name, taxonomy_id, ..., fraction_total_reads per sample: keep the top species
    const [n, frac] = [col(bracken, 'name'), col(bracken, 'fraction_total_reads')];
    const s = col(bracken, 'sample');
    if (n >= 0 && frac >= 0 && s >= 0) {
      const best = new Map<string, [string, number]>();
      for (const r of bracken.rows) {
        const v = parseFloat(r[frac]);
        if (!(v >= 0)) continue;
        const cur = best.get(r[s]);
        if (!cur || v > cur[1]) best.set(r[s], [r[n], v]);
      }
      for (const [smp, [name, v]] of best) row(smp).species = `${name} (${Math.round(v * 100)}%)`;
    }
  }
  return [...rows.values()];
}

/** Friendly titles for the summary tables. */
export const TABLE_TITLES: Record<string, string> = {
  'mlst_output.tsv': 'MLST',
  'sistr_output.tsv': 'SISTR serovar',
  'emmtyper_output.tsv': 'emm type',
  'abricate_output.tsv': 'ABRicate genes',
  'quast_output.tsv': 'Assembly quality (QUAST)',
  'checkm_depth_output.tsv': 'Completeness, contamination and depth',
  'bracken_output.tsv': 'Species abundance (Bracken)',
};
