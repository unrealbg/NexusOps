import { readFile } from 'node:fs/promises';
import { join } from 'node:path';
import { REPOSITORY_ROOT } from './release-common.mjs';

const FILES = Object.freeze({
  signing: '.github/workflows/signed-updater-candidate.yml',
  draft: '.github/workflows/prepare-updater-release-draft.yml',
  publish: '.github/workflows/publish-updater-release.yml',
});

function fail(message) {
  throw new Error(message);
}

function requireManualOnly(source, label) {
  if (!/^on:\s*\n  workflow_dispatch:/m.test(source)) fail(`${label} must use workflow_dispatch`);
  if (/^  (?:push|pull_request|schedule|workflow_run|release|create):/m.test(source))
    fail(`${label} has an automatic trigger`);
}

function requirePinnedActions(source, label) {
  for (const match of source.matchAll(/^\s*- uses: ([^\s]+)$/gm)) {
    if (!/@[0-9a-f]{40}$/.test(match[1])) fail(`${label} uses a floating action reference`);
  }
}

function requireInputsOnlyViaEnvironment(source, label) {
  const lines = source.split(/\r?\n/);
  let runIndent = null;
  for (const line of lines) {
    const run = /^(\s*)(?:-\s+)?run:\s*(.*)$/.exec(line);
    if (run) {
      runIndent = run[1].length;
      if (/\$\{\{\s*inputs\./.test(run[2]))
        fail(`${label} interpolates a workflow input directly into a run command`);
      continue;
    }
    if (runIndent === null || line.trim() === '') continue;
    const indentation = /^\s*/.exec(line)[0].length;
    if (indentation <= runIndent) {
      runIndent = null;
      continue;
    }
    if (/\$\{\{\s*inputs\./.test(line))
      fail(`${label} interpolates a workflow input directly into a run command`);
  }
}

function inputRequired(source, name, label) {
  const block = new RegExp(`^      ${name}:\\s*\\n((?:        .+\\n)+)`, 'm').exec(source)?.[1];
  if (!block || !/^        required: true$/m.test(block)) fail(`${label} lacks required ${name}`);
}

export function validateReleaseWorkflowPolicy({ signing, draft, publish }) {
  for (const [label, source] of Object.entries({ signing, draft, publish })) {
    if (typeof source !== 'string') fail(`${label} workflow source is missing`);
    requireManualOnly(source, label);
    requirePinnedActions(source, label);
    requireInputsOnlyViaEnvironment(source, label);
  }

  for (const name of ['expected_source_sha', 'expected_version']) inputRequired(signing, name, 'signing workflow');
  if (!/^permissions:\s*\n  contents: read\s*\n  id-token: write\s*\n  attestations: write\s*$/m.test(signing))
    fail('signing workflow permissions changed');
  if (/^  (?:contents|actions|administration): write$/m.test(signing))
    fail('signing workflow gained mutation authority');
  if (!/release-identity\.mjs[\s\S]*--expected-source-sha[\s\S]*--expected-version/.test(signing))
    fail('signing workflow identity assertions are missing');
  const signerCommands = [...signing.matchAll(/signer sign --app-version ([^\s]+) "\$STAGED_ARTIFACT"/g)];
  if (signerCommands.length !== 3 || signerCommands.some((match) => match[1] !== '"$PRODUCT_VERSION"'))
    fail('signing workflow does not use the verified product version exactly');
  if (/--app-version\s+"?v?\d+\.\d+\.\d+\b/.test(signing))
    fail('signing workflow hard-codes a production version');
  if (/--app-version[^\r\n]*inputs\./.test(signing))
    fail('signing workflow passes a workflow input directly to the signer');
  if ((signing.match(/PRODUCT_VERSION: \$\{\{ steps\.identity\.outputs\.product_version \}\}/g) ?? []).length !== 3)
    fail('signing workflow version output binding is incomplete');

  const signingSteps = signing.split(/^      - /m).slice(1);
  const secretSteps = signingSteps.filter((step) => /TAURI_SIGNING_PRIVATE_KEY/.test(step));
  if (
    secretSteps.length !== 3 ||
    secretSteps.some((step) => !/^name: Sign staged /m.test(step) || !/signer sign/.test(step))
  ) {
    fail('signing secrets escaped detached signer steps');
  }

  for (const name of ['candidate_run_id', 'expected_source_sha', 'expected_version'])
    inputRequired(draft, name, 'draft workflow');
  if (!/^permissions:\s*\n  actions: read\s*\n  contents: write\s*$/m.test(draft))
    fail('draft workflow permissions changed');
  if (/TAURI_SIGNING_PRIVATE_KEY|id-token: write|attestations: write|administration: write/.test(draft))
    fail('draft workflow mixes signing or administration authority');
  if (!/release-identity\.mjs[\s\S]*prepare-publication-set\.mjs[\s\S]*draft-download[\s\S]*verify-draft-release\.mjs/.test(draft))
    fail('draft workflow verification chain is incomplete');
  if (!/A release already exists for the exact tag/.test(draft))
    fail('draft workflow does not reject an existing release');
  if (!/r\.assets\.length !== 0/.test(draft))
    fail('draft workflow does not require a new empty release');
  if (/--clobber|--overwrite|\bforce\b/i.test(draft)) fail('draft workflow permits asset replacement');

  for (const name of ['release_id', 'candidate_run_id', 'expected_tag', 'expected_source_sha', 'expected_version', 'authenticode_decision'])
    inputRequired(publish, name, 'publish workflow');
  if (!/^permissions:\s*\n  actions: read\s*\n  contents: write\s*$/m.test(publish))
    fail('publish workflow permissions changed');
  if (/TAURI_SIGNING_PRIVATE_KEY|id-token: write|attestations: write|administration: write/.test(publish))
    fail('publish workflow mixes signing or administration authority');
  if (!/immutable-releases[\s\S]*verify-draft-release\.mjs[\s\S]*--immutable-json/.test(publish))
    fail('publish workflow lacks the immutable-release gate');
  if (!/verify-candidate-run\.mjs/.test(publish)) fail('publish workflow does not rebind candidate authority');
  if (!/--method PATCH[\s\S]*-F draft=false -F prerelease=false -f make_latest=true/.test(publish))
    fail('publish workflow activation is missing or changed');
  if (!/activation_status=\$\?[\s\S]*activation-observed-state\.json[\s\S]*no retry was attempted/.test(publish))
    fail('publish workflow lacks ambiguous activation-state handling');
  if (/gh release upload|prepare-publication-set|create-latest-json|signer sign|tauri(?:\.cmd)?\s+(?:build|bundle)|git\s+(?:tag|push)|\/git\/refs/.test(publish))
    fail('publish workflow contains build, signing, tag or asset mutation');
  if (/repos\/[^\s]+\/immutable-releases[^\r\n]*(?:--method|PATCH|POST|PUT|DELETE)/.test(publish))
    fail('publish workflow mutates immutable-release settings');
  return { signingCommands: signerCommands.length, workflows: 3 };
}

export async function verifyReleaseWorkflowPolicy(root = REPOSITORY_ROOT) {
  const entries = await Promise.all(
    Object.entries(FILES).map(async ([key, path]) => [key, await readFile(join(root, path), 'utf8')]),
  );
  return validateReleaseWorkflowPolicy(Object.fromEntries(entries));
}
