// Workflow builder logic, kept apart from the screen: wiring, plain-language
// problems with a fix, adding and removing steps, automatic layout.

import type { Step, Tool, Workflow } from './api';

export const INPUT = 'input';

/** What a source gives: the reads, or a step's output. */
export function sourceType(wf: Workflow, tools: Record<string, Tool>, src: string): string | null {
  if (src === INPUT) return 'reads';
  const [sid, out] = src.split('.');
  const step = wf.steps[sid];
  const tool = step && tools[step.tool];
  return tool?.outputs[out]?.type ?? null;
}

export function stepTool(tools: Record<string, Tool>, step: Step) {
  return tools[step.tool];
}

/** Outputs anywhere in the workflow that fit an input of this type (reads also from the input). */
export function candidates(wf: Workflow, tools: Record<string, Tool>, forStep: string, type: string): string[] {
  const out: string[] = [];
  if (type === 'reads') out.push(INPUT);
  const below = downstream(wf, forStep);
  for (const [sid, st] of Object.entries(wf.steps)) {
    if (sid === forStep || below.has(sid)) continue; // never make a loop
    const tool = tools[st.tool];
    for (const [name, o] of Object.entries(tool?.outputs ?? {})) if (o.type === type) out.push(`${sid}.${name}`);
  }
  // prefer a cleaned/assembled source over the raw reads (fastp.reads before input)
  return out.sort((a, b) => (a === INPUT ? 1 : 0) - (b === INPUT ? 1 : 0));
}

/** Steps that read (directly or not) from this one. */
export function downstream(wf: Workflow, sid: string): Set<string> {
  const seen = new Set<string>();
  const visit = (id: string) => {
    for (const [other, st] of Object.entries(wf.steps)) {
      if (seen.has(other)) continue;
      const reads = Object.values(st.inputs ?? {}).flat().some((s) => s.split('.')[0] === id);
      if (reads) {
        seen.add(other);
        visit(other);
      }
    }
  };
  visit(sid);
  return seen;
}

export function label(wf: Workflow, tools: Record<string, Tool>, src: string) {
  if (src === INPUT) return 'the reads';
  const [sid, out] = src.split('.');
  const st = wf.steps[sid];
  return `${st?.label ?? sid}${out && Object.keys(tools[st?.tool]?.outputs ?? {}).length > 1 ? ` (${out})` : ''}`;
}

export interface Problem {
  step: string;
  input?: string;
  message: string;
  /** the one-click fix: connect this source */
  fix?: string;
}

const TYPE_WORDS: Record<string, string> = {
  reads: 'reads',
  fasta: 'contigs (an assembly)',
  tsv: 'a table',
  txt: 'a report',
  log: 'a log',
};

export function problems(wf: Workflow, tools: Record<string, Tool>): Problem[] {
  const out: Problem[] = [];
  for (const [sid, st] of Object.entries(wf.steps)) {
    const tool = tools[st.tool];
    const name = st.label ?? sid;
    if (!tool) {
      out.push({ step: sid, message: `${name} uses the tool "${st.tool}", which is not installed. Install its tool pack on the Tools page.` });
      continue;
    }
    for (const [inp, spec] of Object.entries(tool.inputs)) {
      const srcs = st.inputs?.[inp] ?? [];
      if (srcs.length || spec.optional) continue;
      const fix = candidates(wf, tools, sid, spec.type)[0];
      const what = TYPE_WORDS[spec.type] ?? spec.type;
      out.push({
        step: sid,
        input: inp,
        message: fix ? `${name} needs ${what}. Connect it to ${label(wf, tools, fix)}, which makes them.` : `${name} needs ${what}, and no step makes them yet. Add one from the tool list.`,
        fix,
      });
    }
  }
  return out;
}

export function slug(text: string) {
  const s = text
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '_')
    .replace(/^_+|_+$/g, '')
    .slice(0, 50);
  return /^[a-z]/.test(s) ? s : `wf_${s || 'new'}`;
}

/** Add a tool as a new step; its inputs are wired to the best matching outputs. */
export function addStep(wf: Workflow, tools: Record<string, Tool>, toolId: string): string {
  const tool = tools[toolId];
  let sid = slug(toolId);
  for (let n = 2; wf.steps[sid]; n++) sid = `${slug(toolId)}_${n}`;
  const step: Step = { tool: toolId, label: tool.name, inputs: {} };
  wf.steps[sid] = step;
  for (const [inp, spec] of Object.entries(tool.inputs)) {
    const c = candidates(wf, tools, sid, spec.type);
    if (c.length && !spec.optional) step.inputs![inp] = spec.type === 'reads' && c.length > 1 ? [c[0], INPUT] : [c[0]];
  }
  return sid;
}

export function removeStep(wf: Workflow, sid: string) {
  delete wf.steps[sid];
  for (const st of Object.values(wf.steps))
    for (const [k, srcs] of Object.entries(st.inputs ?? {})) st.inputs![k] = srcs.filter((s) => s.split('.')[0] !== sid);
  if (wf.layout) delete wf.layout[sid];
}

/** Columns left to right by how deep a step is (at most 5 boxes per column,
 * then a second column beside it); saved positions win. `height` = a box's height. */
export function layout(wf: Workflow, height: (id: string) => number = () => 90): Record<string, { x: number; y: number }> {
  const depth: Record<string, number> = { [INPUT]: 0 };
  const d = (sid: string, stack: string[] = []): number => {
    if (sid in depth) return depth[sid];
    if (stack.includes(sid)) return 1;
    const st = wf.steps[sid];
    let best = 1;
    for (const s of Object.values(st?.inputs ?? {}).flat()) {
      const up = s.split('.')[0];
      if (up === INPUT) continue;
      if (wf.steps[up]) best = Math.max(best, d(up, [...stack, sid]) + 1);
    }
    return (depth[sid] = best);
  };
  Object.keys(wf.steps).forEach((s) => d(s));
  const byCol: string[][] = [];
  for (const id of [INPUT, ...Object.keys(wf.steps)]) (byCol[depth[id]] ??= []).push(id);
  const PER = 5;
  const W = 250;
  const GAP = 28;
  const pos: Record<string, { x: number; y: number }> = {};
  let x = 0;
  for (const ids of byCol.filter(Boolean)) {
    const subCols = Math.ceil(ids.length / PER);
    for (let c = 0; c < subCols; c++) {
      const part = ids.slice(c * PER, (c + 1) * PER);
      const total = part.reduce((s, id) => s + height(id) + GAP, -GAP);
      let y = -total / 2; // centre each column on the same line
      for (const id of part) {
        pos[id] = wf.layout?.[id] ?? { x: x + c * W, y };
        y += height(id) + GAP;
      }
    }
    x += subCols * W + 40;
  }
  return pos;
}
