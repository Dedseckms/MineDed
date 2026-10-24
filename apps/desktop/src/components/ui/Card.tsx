import { type HTMLMotionProps, motion } from "motion/react";
import type { ReactNode } from "react";

import { useMotionPrefs } from "../../hooks/useAppearance";
import { INSTANT } from "../../lib/motion";

export const cardVariants = {
  hidden: { opacity: 0, y: 8 },
  show: { opacity: 1, y: 0 },
};

interface CardProps extends HTMLMotionProps<"div"> {
  children: ReactNode;
}

/** Surface card; animates in as part of a `CardList` (stagger). */
export function Card({ className = "", children, ...rest }: CardProps) {
  const { reduced } = useMotionPrefs();
  return (
    <motion.div
      variants={cardVariants}
      transition={reduced ? INSTANT : { duration: 0.22, ease: "easeOut" }}
      className={`rounded-xl border border-border bg-surface p-4 ${className}`}
      {...rest}
    >
      {children}
    </motion.div>
  );
}

/** Container that staggers the appearance of its `Card` children. */
export function CardList({ className = "", children }: { className?: string; children: ReactNode }) {
  const { reduced } = useMotionPrefs();
  return (
    <motion.div
      initial={reduced ? false : "hidden"}
      animate="show"
      variants={{ hidden: {}, show: { transition: { staggerChildren: 0.04 } } }}
      className={className}
    >
      {children}
    </motion.div>
  );
}

export function Section({ title, children, actions }: { title: ReactNode; children: ReactNode; actions?: ReactNode }) {
  return (
    <section className="flex flex-col gap-4 rounded-xl border border-border bg-surface p-5">
      <header className="flex flex-wrap items-center justify-between gap-3">
        <h2 className="text-base font-semibold tracking-tight">{title}</h2>
        {actions}
      </header>
      {children}
    </section>
  );
}
