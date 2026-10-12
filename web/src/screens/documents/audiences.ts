// #1349 V3-documents (design §A D-A2): what differs between the owner's page and the staff page.

export interface Audience {
  /** The view this page reads. */
  view: 'documents' | 'staff_documents';
  /** Whether the page names a branch: the staff page never does (its view answers for the staff's own). */
  sendsBranch: boolean;
  /** Whether each file carries v2's open-in-a-new-tab and download links. */
  showDownload: boolean;
  /** The link back, and its text key. */
  backHref: (branch: string) => string;
  backText: 'backOwner' | 'backStaff';
  /** Where a refusal sends the browser; null = the refusal is a sentence on the page (D-A3, D-A4). */
  refusalGoesTo: string | null;
}

export const OWNER: Audience = {
  view: 'documents',
  sendsBranch: true,
  showDownload: true,
  backHref: (branch) => `/?branch=${encodeURIComponent(branch)}#transactions`,
  backText: 'backOwner',
  refusalGoesTo: null,
};

export const STAFF: Audience = {
  view: 'staff_documents',
  sendsBranch: false,
  showDownload: false,
  backHref: () => '/staff',
  backText: 'backStaff',
  refusalGoesTo: '/staff',
};
