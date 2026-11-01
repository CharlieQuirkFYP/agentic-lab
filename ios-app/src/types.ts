export type Status = "transcribing" | "awaiting_review" | "generating" | "completed" | "failed" | "cancelled";

export type Attachment = {
  uri: string;
  name: string;
  mimeType?: string;
  size?: number;
};

export type Turn = {
  id: string;
  status: Status;
  transcript: string;
  approvedText: string | null;
  reply: string;
  error?: string;
  attachments: Attachment[];
};

export type Readiness = {
  stt: { name: string; ready: boolean };
  reply: { name: string; ready: boolean };
  current_turn?: {
    turn_id: string;
    status: Status;
    transcript?: string;
    approved_text?: string | null;
    reply?: string;
    error?: { message?: string } | null;
  } | null;
};
