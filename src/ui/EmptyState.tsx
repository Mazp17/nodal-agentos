import type { ReactNode } from "react";
import { BrandMark } from "./BrandMark";
import "./empty-state.css";

export interface EmptyStateProps {
  title: ReactNode;
  description?: string;
  /** Buttons under the text. */
  children?: ReactNode;
  className?: string;
}

/** Centered state for an empty page or panel: brand mark, title, text and actions over an accent glow. */
export function EmptyState({ title, description, children, className }: EmptyStateProps) {
  return (
    <div className={`empty-state ${className ?? ""}`}>
      <div className="empty-state-body">
        <BrandMark size={44} />
        <h2 className="empty-state-title">{title}</h2>
        {description && <p className="empty-state-text">{description}</p>}
        {children && <div className="empty-state-actions">{children}</div>}
      </div>
    </div>
  );
}
