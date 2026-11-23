// Icon paths are from Lucide (https://lucide.dev), used under the ISC licence:
//
// ISC License
// Copyright (c) for portions of Lucide are held by Cole Bemis 2013-2022 as part of Feather (MIT).
// All other copyright (c) for Lucide are held by Lucide Contributors 2022.
// Permission to use, copy, modify, and/or distribute this software for any purpose with or without
// fee is hereby granted, provided that the above copyright notice and this permission notice appear
// in all copies. THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES.
//
// Inlined as 24x24, stroke 2, round caps and joins; the calendar day number is our own addition.
import type { ComponentChildren } from "preact";

function Svg({ children, size = 26 }: { children: ComponentChildren; size?: number }) {
  return (
    <svg
      xmlns="http://www.w3.org/2000/svg"
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="2"
      stroke-linecap="round"
      stroke-linejoin="round"
      aria-hidden="true"
    >
      {children}
    </svg>
  );
}

export function MailboxIcon({ size }: { size?: number }) {
  return (
    <Svg size={size}>
      <path d="M22 17a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V9.5C2 7 4 5 6.5 5H18c2.2 0 4 1.8 4 4v8Z" />
      <polyline points="15,9 18,9 18,11" />
      <path d="M6.5 5C9 5 11 7 11 9.5V17a2 2 0 0 1-2 2" />
      <line x1="6" x2="7" y1="10" y2="10" />
    </Svg>
  );
}

/** A calendar with the day of the month inside, like the iOS Calendar icon. */
export function CalendarDayIcon({ day, size }: { day: number | string; size?: number }) {
  return (
    <Svg size={size}>
      <path d="M8 2v4" />
      <path d="M16 2v4" />
      <rect width="18" height="18" x="3" y="4" rx="2" />
      <path d="M3 10h18" />
      <text
        x="12"
        y="18"
        text-anchor="middle"
        font-size="8"
        font-weight="700"
        fill="currentColor"
        stroke="none"
      >
        {day}
      </text>
    </Svg>
  );
}

/** A bulleted list: the Views tab, and the empty state of views without their own icon. */
export function ListIcon({ size }: { size?: number }) {
  return (
    <Svg size={size}>
      <path d="M8 6h13" />
      <path d="M8 12h13" />
      <path d="M8 18h13" />
      <path d="M3 6h.01" />
      <path d="M3 12h.01" />
      <path d="M3 18h.01" />
    </Svg>
  );
}

/** A gear: the Settings button on the Views screen. */
export function GearIcon() {
  return (
    <Svg size={24}>
      <path d="M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z" />
      <circle cx="12" cy="12" r="3" />
    </Svg>
  );
}

export function ChevronRightIcon() {
  return (
    <Svg size={16}>
      <path d="m9 18 6-6-6-6" />
    </Svg>
  );
}

export function ChevronLeftIcon() {
  return (
    <Svg size={22}>
      <path d="m15 18-6-6 6-6" />
    </Svg>
  );
}

export function PlusIcon() {
  return (
    <Svg>
      <path d="M5 12h14" />
      <path d="M12 5v14" />
    </Svg>
  );
}

export function NoteIcon() {
  return (
    <Svg size={14}>
      <path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z" />
      <path d="M14 2v6h6" />
      <path d="M8 13h8" />
      <path d="M8 17h8" />
    </Svg>
  );
}

export function RepeatIcon() {
  return (
    <Svg size={14}>
      <path d="m17 2 4 4-4 4" />
      <path d="M3 11v-1a4 4 0 0 1 4-4h14" />
      <path d="m7 22-4-4 4-4" />
      <path d="M21 13v1a4 4 0 0 1-4 4H3" />
    </Svg>
  );
}

export function ProjectIcon() {
  return (
    <Svg size={14}>
      <path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z" />
    </Svg>
  );
}

export function ChecklistIcon() {
  return (
    <Svg size={14}>
      <path d="m3 17 2 2 4-4" />
      <path d="m3 7 2 2 4-4" />
      <path d="M13 6h8" />
      <path d="M13 12h8" />
      <path d="M13 18h8" />
    </Svg>
  );
}

export function AlertIcon() {
  return (
    <Svg size={14}>
      <path d="m21.73 18-8-14a2 2 0 0 0-3.48 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.73-3" />
      <path d="M12 9v4" />
      <path d="M12 17h.01" />
    </Svg>
  );
}

/** An iOS-style activity indicator: eight fading spokes, turned in steps by CSS (our own). */
export function SpinnerIcon() {
  return (
    <svg class="spinner" viewBox="0 0 24 24" aria-hidden="true">
      {Array.from({ length: 8 }, (_, i) => (
        <line
          key={i}
          x1="12"
          y1="2.5"
          x2="12"
          y2="7"
          stroke="currentColor"
          stroke-width="2.4"
          stroke-linecap="round"
          opacity={1 - i * 0.11}
          transform={`rotate(${-i * 45} 12 12)`}
        />
      ))}
    </svg>
  );
}
