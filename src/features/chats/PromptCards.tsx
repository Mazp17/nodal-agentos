import { useState, type KeyboardEvent } from "react";
import { MessageCircleQuestion, Shield, ShieldAlert } from "lucide-react";
import { respondChatPermission, type PermissionRequest } from "../../domain/api";
import { useToast } from "../../ui/Toasts";
import { readQuestions, toolLabel, type Question } from "./model";
import "./prompt-cards.css";

const OTHER = "\u0000other";

/** The queue of Claude's prompts: the first as a full card, the rest as quiet "Next" rows. */
export function PendingPrompts({ chatId, pending }: { chatId: string; pending: PermissionRequest[] }) {
  const [first, ...rest] = pending;
  if (!first) return null;
  return (
    <div className="pc-queue">
      <PromptCard key={first.requestId} chatId={chatId} req={first} />
      {rest.map((r) => (
        <div key={r.requestId} className="pc-next">
          <span className="pc-next-text ellipsis">Next: {nextLabel(r)}</span>
          <span className="pc-next-after">after this</span>
        </div>
      ))}
    </div>
  );
}

function nextLabel(req: PermissionRequest): string {
  const qs = readQuestions(req);
  if (qs) {
    const heads = qs.map((q) => q.header).filter(Boolean).join(", ");
    return `Claude has ${qs.length} question${qs.length === 1 ? "" : "s"}${heads ? ` · ${heads}` : ""}`;
  }
  return `Allow ${toolLabel(req.toolName)}${req.summary ? ` · ${req.summary}` : ""}`;
}

function PromptCard({ chatId, req }: { chatId: string; req: PermissionRequest }) {
  const questions = readQuestions(req);
  return questions ? <QuestionCard chatId={chatId} req={req} questions={questions} /> : <ApprovalCard chatId={chatId} req={req} />;
}

// ---------- Permission ----------

export function ApprovalCard({ chatId, req }: { chatId: string; req: PermissionRequest }) {
  const toast = useToast();
  const [note, setNote] = useState("");
  const [busy, setBusy] = useState(false);
  const [showInput, setShowInput] = useState(false);
  const [denying, setDenying] = useState(false);
  const input = req.input == null ? null : JSON.stringify(req.input, null, 2);
  const label = toolLabel(req.toolName);

  const answer = (allow: boolean) => {
    setBusy(true);
    respondChatPermission(chatId, req.requestId, allow, allow ? null : note.trim() || null).then(
      // The card goes away with `permissionResolved`.
      () => {},
      (e: unknown) => {
        setBusy(false);
        toast("Couldn't answer the prompt", String(e), "danger");
      },
    );
  };

  return (
    <div className="pc-card pc-perm" role="group" aria-label={`Permission request: ${label}`}>
      <div className="pc-perm-head">
        {denying ? <Shield size={18} aria-hidden focusable="false" /> : <ShieldAlert size={18} aria-hidden focusable="false" />}
        <h3 className="pc-perm-title">{req.description ? `Allow ${label}: ${req.description}` : `Allow ${label}?`}</h3>
      </div>
      {req.summary && <code className="pc-cmd">{req.summary}</code>}
      {input && (
        <button type="button" className="pc-link" aria-expanded={showInput} onClick={() => setShowInput(!showInput)}>
          Full input {showInput ? "▾" : "▸"}
        </button>
      )}
      {showInput && input && <pre className="tr-pre pc-input">{input}</pre>}
      {denying ? (
        <div className="pc-actions">
          <input
            className="input pc-note"
            value={note}
            autoFocus
            placeholder="Why? Claude reads this (optional)"
            aria-label="Reason for denying"
            disabled={busy}
            onChange={(e) => setNote(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !e.nativeEvent.isComposing) {
                e.preventDefault();
                answer(false);
              }
            }}
          />
          <button type="button" className="btn btn-sm" disabled={busy} onClick={() => setDenying(false)}>
            Cancel
          </button>
          <button type="button" className="btn btn-sm btn-danger" disabled={busy} onClick={() => answer(false)}>
            Deny
          </button>
        </div>
      ) : (
        <div className="pc-actions pc-actions-end">
          <button type="button" className="btn btn-sm" disabled={busy} onClick={() => setDenying(true)}>
            Deny…
          </button>
          <button type="button" className="btn btn-sm btn-amber" disabled={busy} onClick={() => answer(true)}>
            Allow
          </button>
        </div>
      )}
    </div>
  );
}

// ---------- Questions ----------

/** Claude's `AskUserQuestion`, one question per step, with a review step when there are several. */
function QuestionCard({ chatId, req, questions }: { chatId: string; req: PermissionRequest; questions: Question[] }) {
  const toast = useToast();
  const n = questions.length;
  const [step, setStep] = useState(0);
  const [picked, setPicked] = useState<string[][]>(() => questions.map(() => []));
  const [other, setOther] = useState<string[]>(() => questions.map(() => ""));
  const [busy, setBusy] = useState(false);

  const answerOf = (i: number) =>
    (picked[i] ?? [])
      .map((l) => (l === OTHER ? (other[i] ?? "").trim() : l))
      .filter(Boolean)
      .join(", ");
  const allDone = questions.every((_, i) => answerOf(i) !== "");
  const reviewing = n > 1 && step === n;
  const q = questions[step];
  const canNext = reviewing ? allDone : answerOf(step) !== "";

  const toggle = (i: number, label: string, multi: boolean) =>
    setPicked((all) =>
      all.map((p, j) => (j !== i ? p : !multi ? [label] : p.includes(label) ? p.filter((l) => l !== label) : [...p, label])),
    );

  const send = (allow: boolean) => {
    setBusy(true);
    const answers = allow ? Object.fromEntries(questions.map((qq, i) => [qq.question, answerOf(i)])) : null;
    respondChatPermission(chatId, req.requestId, allow, allow ? null : "The user chose not to answer.", answers).then(
      // The card goes away with `permissionResolved`.
      () => {},
      (e: unknown) => {
        setBusy(false);
        toast("Couldn't send the answer", String(e), "danger");
      },
    );
  };

  const advance = () => {
    if (!canNext || busy) return;
    if (n === 1 || reviewing) send(true);
    else setStep(step + 1);
  };

  const onKeyDown = (e: KeyboardEvent) => {
    const t = e.target as HTMLInputElement;
    if (reviewing || !q || (t.tagName === "INPUT" && t.type === "text")) return;
    const k = Number(e.key);
    if (Number.isInteger(k) && k >= 1 && k <= q.options.length) {
      e.preventDefault();
      toggle(step, q.options[k - 1]!.label, q.multiSelect);
    }
  };

  const tabCls = (on: boolean) => `pc-tab${on ? " pc-tab-on" : ""}`;
  const nextText = reviewing || n === 1 ? (n === 1 ? "Send" : "Send answers") : step === n - 1 ? "Review" : "Next";

  return (
    <form
      className="pc-card pc-ask"
      aria-label="Claude has questions"
      onKeyDown={onKeyDown}
      onSubmit={(e) => {
        e.preventDefault();
        advance();
      }}
    >
      <div className="pc-ask-head">
        <MessageCircleQuestion size={18} aria-hidden focusable="false" />
        <span>
          Claude has {n} question{n === 1 ? "" : "s"} before it goes on
        </span>
        {n > 1 && <span className="pc-ask-count num">{reviewing ? "Review" : `${step + 1} of ${n}`}</span>}
      </div>

      {n > 1 && (
        <div className="pc-tabs" role="tablist" aria-label="Questions">
          {questions.map((qq, i) => {
            const a = answerOf(i);
            const head = qq.header ?? `Question ${i + 1}`;
            return (
              <button key={i} type="button" role="tab" aria-selected={step === i} className={tabCls(step === i)} disabled={busy} onClick={() => setStep(i)}>
                {a ? `${head} · ${a}` : head}
              </button>
            );
          })}
          <button type="button" role="tab" aria-selected={reviewing} className={`${tabCls(reviewing)} pc-tab-review`} disabled={busy || !allDone} onClick={() => setStep(n)}>
            Review
          </button>
        </div>
      )}

      {reviewing ? (
        <div className="pc-review">
          <div className="pc-legend">Send these answers?</div>
          {questions.map((qq, i) => (
            <div key={i} className="pc-review-row">
              <span className="pc-review-head">{qq.header ?? `Question ${i + 1}`}</span>
              <span className="pc-review-ans">{answerOf(i)}</span>
              <button type="button" className="pc-link pc-link-info" disabled={busy} onClick={() => setStep(i)}>
                Change
              </button>
            </div>
          ))}
        </div>
      ) : (
        q && (
          <fieldset key={step} className="pc-set" disabled={busy}>
            <legend className="pc-legend">{q.question}</legend>
            <div className="pc-hint">{q.multiSelect ? "Pick one or more" : "Pick one"}</div>
            {q.options.map((o, i) => {
              const on = (picked[step] ?? []).includes(o.label);
              return (
                <label key={o.label} className={`pc-opt${on ? " pc-opt-on" : ""}`}>
                  <input type={q.multiSelect ? "checkbox" : "radio"} name={`${req.requestId}-${step}`} checked={on} onChange={() => toggle(step, o.label, q.multiSelect)} />
                  <span className="pc-opt-body">
                    <span className="pc-opt-label">{o.label}</span>
                    {o.description && <span className="pc-opt-desc">{o.description}</span>}
                  </span>
                  {i < 9 && <kbd className="pc-kbd">{i + 1}</kbd>}
                </label>
              );
            })}
            <label className={`pc-opt${(picked[step] ?? []).includes(OTHER) ? " pc-opt-on" : ""}`}>
              <input
                type={q.multiSelect ? "checkbox" : "radio"}
                name={`${req.requestId}-${step}`}
                aria-label="Other answer"
                checked={(picked[step] ?? []).includes(OTHER)}
                onChange={() => toggle(step, OTHER, q.multiSelect)}
              />
              <span className="pc-opt-desc">Other</span>
              <input
                className="pc-other"
                value={other[step] ?? ""}
                placeholder="Write your own answer"
                aria-label={`Other answer to: ${q.question}`}
                onFocus={() => !(picked[step] ?? []).includes(OTHER) && toggle(step, OTHER, q.multiSelect)}
                onChange={(e) => setOther((all) => all.map((v, j) => (j === step ? e.target.value : v)))}
              />
            </label>
          </fieldset>
        )
      )}

      <div className="pc-foot">
        <button type="button" className="pc-quiet" disabled={busy} title="Claude goes on without your answers" onClick={() => send(false)}>
          Don't answer
        </button>
        <span className="pc-foot-gap" />
        {n > 1 && step > 0 && (
          <button type="button" className="btn btn-sm" disabled={busy} onClick={() => setStep(step - 1)}>
            Back
          </button>
        )}
        <button type="submit" className="btn btn-sm pc-next-btn" disabled={busy || !canNext}>
          {nextText}
        </button>
      </div>
    </form>
  );
}
