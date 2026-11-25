// One namespace per file, named after it. Listed by hand: React Native has no import.meta.glob.

import accounts from "./accounts";
import auth from "./auth";
import chat from "./chat";
import common from "./common";
import errors from "./errors";
import follows from "./follows";
import friends from "./friends";
import map from "./map";
import notifications from "./notifications";
import profile from "./profile";
import shorts from "./shorts";
import stories from "./stories";

export default {
  common,
  errors,
  auth,
  accounts,
  map,
  friends,
  follows,
  chat,
  notifications,
  stories,
  shorts,
  profile,
};
