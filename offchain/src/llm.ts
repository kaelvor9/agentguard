// NEAR AI Cloud client. OpenAI-compatible API; the default model runs inside NEAR AI's TEEs.
import "dotenv/config";
import OpenAI from "openai";

const apiKey = process.env.NEAR_AI_API_KEY;
export const baseURL = "https://cloud-api.near.ai/v1";
// Canonical ID of a NEAR-hosted, attested model (verifiable: true in /v1/model/list).
// Use canonical IDs, not aliases: NEAR's verification guide requires it.
export const model = process.env.NEAR_AI_MODEL || "z-ai/glm-5.3-flash";
export const aiEnabled = Boolean(apiKey);

const client = aiEnabled
  ? new OpenAI({
      apiKey,
      baseURL,
      timeout: 30_000, // never let a slow model stall the guardian
      maxRetries: 1,
      defaultHeaders: { "x-no-aliasing": "true" }, // fail rather than silently switch models
    })
  : null;

export interface AiResult<T> {
  value: T;
  chatId: string; // completion id: lets the owner fetch NEAR AI's signature for this answer
}

/** Pulls the last flat JSON object out of a reply, ignoring any reasoning text around it. */
export function extractJson<T>(text: string): T | null {
  const cleaned = text.replace(/<think>[\s\S]*?<\/think>/gi, "");
  const matches = cleaned.match(/\{[^{}]*\}/g);
  if (!matches) return null;
  for (const candidate of matches.reverse()) {
    try {
      return JSON.parse(candidate) as T;
    } catch {
      // try the previous one
    }
  }
  return null;
}

/**
 * Asks the model for a JSON object. Returns null if AI is off or no usable answer arrives.
 * Reasoning models can spend many tokens thinking before they answer, so the budget is generous,
 * and an unreadable answer is logged (with why the model stopped) and retried once.
 */
export async function askJson<T>(system: string, user: string): Promise<AiResult<T> | null> {
  if (!client) return null;
  for (let attempt = 1; attempt <= 2; attempt++) {
    try {
      const res = await client.chat.completions.create({
        model,
        temperature: 0,
        max_tokens: 2000,
        messages: [
          { role: "system", content: system },
          { role: "user", content: user },
        ],
      });
      const choice = res.choices[0];
      const content = choice?.message?.content ?? "";
      const value = extractJson<T>(content);
      if (value) return { value, chatId: res.id };
      const snippet = content.replace(/\s+/g, " ").slice(-160) || "(empty)";
      console.warn(`  [ai] attempt ${attempt}: no JSON in answer (stopped: ${choice?.finish_reason ?? "?"}): ...${snippet}`);
    } catch (err) {
      console.warn(`  [ai] attempt ${attempt} failed: ${(err as Error).message.slice(0, 200)}`);
    }
  }
  return null;
}
