// Dev-only: a sample chat with every element the conversation renders, built from fixtures.

import { PROPOSE_TASK_TOOL, type PermissionRequest } from "../../domain/api";
import type { Repo } from "../../domain/types";
import { Conversation } from "../chats/Conversation";
import { toTurns } from "../chats/model";
import type { ChatStream } from "../chats/stream";
import type { TranscriptItem } from "../chats/types";
import "../chats/transcript.css";
import "../chats/chat.css";

const REPO: Repo = { id: "debug-repo", projectId: "debug-project", path: "/tmp/debug-repo", name: "debug-repo", position: 0, createdAt: 0 };

const LONG_THOUGHT = Array.from(
  { length: 20 },
  (_, i) => `${i + 1}. Checking step ${i + 1} of the plan: read the caller, compare it with \`reduce\`, and note what changes.`,
).join("\n");

const ok = (text: string) => ({ text, isError: false, truncated: false, patch: null });

const ITEMS: TranscriptItem[] = [
  { kind: "user", text: "Add a logout button", truncated: false, uuid: "u1" },
  { kind: "user", text: "[Request interrupted by user]", truncated: false },
  { kind: "user", text: "Add a logout button to the header, and make sure the build still passes.", truncated: false, uuid: "u2" },
  { kind: "thinking", text: "I need to find the **header** first. Plan:\n\n- look at the layout\n- add the button\n- run the build", truncated: false },
  { kind: "text", text: "I'll look at how the header is built first.", truncated: false },
  {
    kind: "toolUse",
    id: "t-agent",
    name: "Agent",
    summary: "Find the header component",
    input: '{ "subagent_type": "Explore", "description": "Find the header component" }',
    result: { ...ok("The header is in src/main.ts."), agent: { agentId: "a1b2c3", agentType: "Explore", status: "completed", durationMs: 42000, totalTokens: 18000, toolUses: 12 } },
  },
  { kind: "toolUse", id: "t-read", name: "Read", summary: "/tmp/debug-repo/src/main.ts", input: '{ "file_path": "/tmp/debug-repo/src/main.ts" }', result: ok("1  export const x = 1;") },
  {
    kind: "toolUse",
    id: "t-edit",
    name: "Edit",
    summary: "/tmp/debug-repo/src/main.ts",
    input: null,
    result: {
      text: "Edited",
      isError: false,
      truncated: false,
      patch: {
        truncated: false,
        file: {
          path: "src/main.ts",
          oldPath: null,
          status: "modified",
          additions: 1,
          deletions: 1,
          binary: false,
          hunks: [
            {
              header: "@@ -1 +1 @@",
              oldStart: 1,
              oldLines: 1,
              newStart: 1,
              newLines: 1,
              lines: [
                { kind: "del", text: "export const x = 1;", oldNo: 1, newNo: null },
                { kind: "add", text: "export const x = 2;", oldNo: null, newNo: 1 },
              ],
            },
          ],
        },
      },
    },
  },
  {
    kind: "toolUse",
    id: "t-ask",
    name: "AskUserQuestion",
    summary: null,
    input: JSON.stringify({ questions: [{ question: "Where should the button go?", header: "Scope", multiSelect: false, options: [{ label: "Header only", description: "" }] }] }),
    result: ok('User has answered your questions: "Where should the button go?"="Header only". You can now continue.'),
  },
  { kind: "toolUse", id: "t-bash", name: "Bash", summary: "pnpm build", input: '{ "command": "pnpm build" }', result: ok("✓ built in 228ms") },
  { kind: "toolUse", id: "t-err", name: "Bash", summary: "pnpm test", input: null, result: { text: "Error: 1 test failed", isError: true, truncated: false, patch: null } },
  { kind: "toolUse", id: "t-denied", name: "Bash", summary: "rm -rf dist", input: null, result: null },
  { kind: "thinking", text: LONG_THOUGHT, truncated: true },
  { kind: "text", text: "The button is in. One test fails for an unrelated reason:\n\n1. `logout` works\n2. `session.spec.ts` was already failing\n\n```ts\nexport const x = 2;\n```\n\nI'd track the failing test as its own [task](https://example.com):", truncated: false },
  {
    kind: "toolUse",
    id: "t-prop",
    name: PROPOSE_TASK_TOOL,
    summary: "Fix the session test",
    input: null,
    result: ok(
      JSON.stringify({
        repoName: REPO.name,
        newTask: { projectId: REPO.projectId, repoId: REPO.id, title: "Fix the failing session test", plan: { kind: "text", text: "## Steps\n1. Reproduce it\n2. Fix it" }, priority: "medium", acceptance: ["session.spec.ts passes"] },
      }),
    ),
  },
  { kind: "user", text: '<agent-message from="a1b2c3">The header is in src/main.ts.</agent-message>', truncated: false },
  { kind: "user", text: "Now clean the build output and run the tests again.", truncated: false, uuid: "u3" },
  { kind: "toolUse", id: "t-agent-run", name: "Agent", summary: "Run the test suite", input: '{ "subagent_type": "general-purpose", "description": "Run the test suite" }', result: null },
  { kind: "toolUse", id: "t-run", name: "Bash", summary: "rm -rf dist", input: null, result: null },
];

const APPROVAL: PermissionRequest = {
  requestId: "r-approval",
  toolName: "Bash",
  toolUseId: "t-run",
  description: "Delete the build output",
  summary: "rm -rf dist",
  input: { command: "rm -rf dist" },
};

const QUESTION: PermissionRequest = {
  requestId: "r-question",
  toolName: "AskUserQuestion",
  toolUseId: null,
  description: null,
  summary: null,
  input: {
    questions: [
      {
        question: "Run the slow e2e tests too?",
        header: "Tests",
        multiSelect: false,
        options: [
          { label: "Unit only", description: "Fast" },
          { label: "Unit and e2e", description: "About 5 minutes" },
        ],
      },
    ],
  },
};

// The chat mid-answer, so the live elements show next to the finished ones.
const STREAM: ChatStream = {
  loaded: true,
  loadError: null,
  omitted: 12,
  items: ITEMS,
  outbox: [],
  streaming: "Cleaning the build output, then I'll run",
  thinking: false,
  thinkingText: "The user wants a clean build first, so `dist` goes before the tests run.",
  state: "busy",
  pending: [APPROVAL, QUESTION],
  denied: { "t-denied": "Denied by the user" },
  subagents: {},
  contextTokens: null,
  contextWindow: null,
  notice: null,
  interrupting: false,
};

export function DebugChat() {
  return (
    <div className="debug-chat">
      <Conversation
        chatId="debug-chat"
        turns={toTurns(STREAM.items, STREAM.outbox)}
        stream={STREAM}
        projectId={REPO.projectId}
        projectKey="DBG"
        repos={[REPO]}
        onOpenTask={() => {}}
        onOpenDiff={() => {}}
        onRewind={() => {}}
        onResend={() => {}}
      />
    </div>
  );
}
