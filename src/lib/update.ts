import { useEffect, useState } from "react";
import { api, onEvent, type UpdateStatus } from "../api";

export function useUpdateStatus(): UpdateStatus | null {
  const [s, setS] = useState<UpdateStatus | null>(null);
  useEffect(() => {
    api.updateStatus().then(setS, () => {});
    return onEvent<UpdateStatus>("update-status", (u) => setS((prev) => ({ ...(prev ?? u), ...u })));
  }, []);
  return s;
}
