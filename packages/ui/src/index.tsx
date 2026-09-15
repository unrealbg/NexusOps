import { useEffect, useRef, type ButtonHTMLAttributes, type ReactNode } from 'react';

export function Button({
  variant = 'secondary',
  className = '',
  ...props
}: ButtonHTMLAttributes<HTMLButtonElement> & {
  variant?: 'primary' | 'secondary' | 'ghost' | 'danger';
}) {
  return <button className={`button button--${variant} ${className}`} {...props} />;
}

export function Badge({
  children,
  tone = 'neutral',
}: {
  children: ReactNode;
  tone?: 'neutral' | 'success' | 'warning' | 'danger';
}) {
  return (
    <span className={`badge badge--${tone}`}>
      <span className="badge-dot" aria-hidden="true" />
      {children}
    </span>
  );
}

/** Native dialog supplies focus containment, Escape support and background inertness. */
export function Modal({
  title,
  children,
  onClose,
  wide = false,
}: {
  title: string;
  children: ReactNode;
  onClose: () => void;
  wide?: boolean;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    const dialog = ref.current;
    const previous = document.activeElement;
    dialog?.showModal();
    dialog?.querySelector<HTMLElement>('[data-initial-focus]')?.focus();
    return () => {
      dialog?.close();
      if (previous instanceof HTMLElement) previous.focus();
    };
  }, []);
  return (
    <dialog
      ref={ref}
      className={`modal ${wide ? 'modal--wide' : ''}`}
      aria-labelledby="dialog-title"
      onCancel={(event) => {
        event.preventDefault();
        onClose();
      }}
    >
      <div className="modal-heading">
        <h2 id="dialog-title">{title}</h2>
        <Button variant="ghost" onClick={onClose} aria-label="Close dialog">
          ×
        </Button>
      </div>
      {children}
    </dialog>
  );
}

export function Notice({
  children,
  tone = 'danger',
}: {
  children: ReactNode;
  tone?: 'danger' | 'warning' | 'neutral';
}) {
  return (
    <div className={`notice notice--${tone}`} role={tone === 'neutral' ? 'status' : 'alert'}>
      {children}
    </div>
  );
}

export function Spinner({ label = 'Loading' }: { label?: string }) {
  return (
    <span className="loading">
      <span className="spinner" aria-hidden="true" />
      <span role="status">{label}</span>
    </span>
  );
}
