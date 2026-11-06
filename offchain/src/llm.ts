// NEAR AI Cloud client. OpenAI-compatible; models run inside TEEs (private inference).
import "dotenv/config";
import OpenAI from "openai";

const apiKey = process.env.NEAR_AI_API_KEY;
export const model = process.env.NEAR_AI_MODEL ?? "zai-org/GLM-5.1-FP8";
export const aiEnabled = Boolean(apiKey);

const client = aiEnabled
  ? new OpenAI({ apiKey, baseURL: "https://cloud-api.near.ai/v1" })
  : null;

/** Asks the model for a JSON object and parses it. Returns null if AI is off or the reply is unusable. */
export async function askJson<T>(system: string, user: string): Promise<T | null> {
  if (!client) return null;
  try {
    const res = await client.chat.completions.create({
      model,
      temperature: 0,
      messages: [
        { role: "system", content: system },
        { role: "user", content: user },
      ],
    });
    const text = res.choices[0]?.message?.content ?? "";
    const match = text.match(/\{[\s\S]*\}/);
    return match ? (JSON.parse(match[0]) as T) : null;
  } catch (err) {
    console.warn(`  [ai] request failed: ${(err as Error).message}`);
    return null;
  }
}
