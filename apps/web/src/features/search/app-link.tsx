import { useRouter } from "@tanstack/react-router";
import type { AnchorHTMLAttributes, MouseEvent } from "react";

/**
 * A link to a page of this app whose address is only known as text: the path
 * a domain published for a search result, or a route another feature owns.
 * It is a real link (open in a new tab, copy the address) that the router
 * follows in place on a plain click.
 */
export function AppLink({ href, onClick, ...props }: Omit<AnchorHTMLAttributes<HTMLAnchorElement>, "href"> & { href: string }) {
  const router = useRouter();
  const follow = (event: MouseEvent<HTMLAnchorElement>) => {
    onClick?.(event);
    if (event.defaultPrevented || event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;
    event.preventDefault();
    void router.navigate({ href });
  };
  return <a href={href} onClick={follow} {...props} />;
}
