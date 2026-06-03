import { evalVdf } from "./vdf.js";

self.onmessage = async (e) => {
  const { seedHex, modulusHex, difficulty } = e.data;
  try {
    const result = await evalVdf(seedHex, modulusHex, difficulty, (p) => {
      self.postMessage({ type: "progress", progress: p });
    });
    self.postMessage({ type: "done", outputHex: result.outputHex, proofHex: result.proofHex });
  } catch (error) {
    self.postMessage({ type: "error", message: error.message });
  }
};