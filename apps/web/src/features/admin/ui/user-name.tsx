import { useQuery } from "@tanstack/react-query";
import { userAdminClient } from "../api";
import { UserCell } from "./user";

/** The account behind an ID that came from the address bar (filter tokens). Falls back to the ID. */
export function UserName({ userId }: { userId: string }) {
  const { data } = useQuery({
    queryKey: ["admin", "user", userId],
    queryFn: () => userAdminClient().getUser({ userId }),
  });
  const user = data?.user?.user;
  return user ? (
    <UserCell user={user} link={false} />
  ) : (
    <code className="font-mono text-xs">{userId}</code>
  );
}
