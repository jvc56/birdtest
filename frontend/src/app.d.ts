declare global {
  namespace App {
    interface PageState {
      /** Set by registration when the backend mails to its stdout log. */
      mailInServerLog?: boolean;
    }
  }
}

export {};
