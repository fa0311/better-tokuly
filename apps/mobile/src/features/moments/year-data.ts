import { useQueries } from "@tanstack/react-query";
import { momentQueries } from "@tokuly/core/moments";
import { momentClient as mobileMomentClient } from "./data";

/** Fetch only elapsed months; the keys are the same ones the calendar screen uses. */
export function useMomentYear(year: number, monthCount: number) {
  return useQueries({
    queries: Array.from({ length: monthCount }, (_, index) => {
      const month = index + 1;
      return {
        ...momentQueries.calendar(mobileMomentClient, year, month),
        staleTime: 60_000,
      };
    }),
  });
}
